use chrono::{Datelike, NaiveDate};

use crate::decision::config::DecisionConfig;
use crate::indicators::options::{
    black_scholes_greeks, black_scholes_price, first_touch_probability, lognormal_probability,
    BlackScholesInput, OptionKind,
};
use crate::indicators::warrant::OpportunitySignal;

#[derive(Debug, Clone)]
pub struct ScenarioConfig {
    pub target_price: f64,
    pub target_date: NaiveDate,
    pub min_maturity: Option<NaiveDate>,
    pub max_maturity: Option<NaiveDate>,
    pub side: String,
    pub risk_free_rate: f64,
    pub dividend_yield: f64,
    pub fallback_volatility: f64,
    pub volatility_shock_points: f64,
    pub real_world_drift: f64,
    pub fx_target_rate: Option<f64>,
    pub fx_stress_pct: f64,
    pub vol_spot_slope_points_per_pct: f64,
    pub exit_spread_multiplier: f64,
    pub exit_spread_delta_penalty: f64,
    pub stale_pricing_guard: bool,
    pub paris_hour: Option<u32>,
    pub dividends: Vec<DiscreteDividend>,
}

#[derive(Debug, Clone)]
pub struct DiscreteDividend {
    pub ex_date: NaiveDate,
    pub amount: f64,
}

#[derive(Debug, Clone)]
pub struct ScenarioCandidate {
    pub decision: &'static str,
    pub score: f64,
    pub symbol: String,
    pub side: String,
    pub product_type: String,
    pub url: String,
    pub strike: f64,
    pub maturity: NaiveDate,
    pub entry_price: f64,
    pub projected_price: f64,
    pub projected_bid_price: f64,
    pub stressed_projected_price: Option<f64>,
    pub fx_stressed_projected_price: Option<f64>,
    pub net_return_pct: f64,
    pub stressed_net_return_pct: Option<f64>,
    pub fx_stressed_net_return_pct: Option<f64>,
    pub gross_return_pct: f64,
    pub fee_drag_pct: f64,
    pub breakeven_underlying: Option<f64>,
    pub breakeven_distance_pct: Option<f64>,
    pub implied_volatility: Option<f64>,
    pub volatility_used: f64,
    pub exit_volatility: f64,
    pub dynamic_volatility_shift_points: f64,
    pub volatility_shock_points: f64,
    pub theta_horizon_pct: Option<f64>,
    pub spread_pct: Option<f64>,
    pub probability_target_pct: Option<f64>,
    pub probability_breakeven_pct: Option<f64>,
    pub terminal_probability_target_pct: Option<f64>,
    pub terminal_probability_breakeven_pct: Option<f64>,
    pub target_zscore: Option<f64>,
    pub breakeven_zscore: Option<f64>,
    pub expected_value_pct: Option<f64>,
    pub sharpe_like: Option<f64>,
    pub kelly_fraction_pct: Option<f64>,
    pub delta: Option<f64>,
    pub target_delta: Option<f64>,
    pub gamma: Option<f64>,
    pub target_gamma: Option<f64>,
    pub vega_per_vol_point: Option<f64>,
    pub target_vega_per_vol_point: Option<f64>,
    pub theta_per_day: Option<f64>,
    pub target_theta_per_day: Option<f64>,
    pub rho_per_rate_point: Option<f64>,
    pub d1: Option<f64>,
    pub d2: Option<f64>,
    pub data_quality_score: f64,
    pub liquidity_score: f64,
    pub parity: f64,
    pub fx_rate: f64,
    pub fx_exit_rate: f64,
    pub fx_stressed_exit_rate: Option<f64>,
    pub effective_exit_spread_multiplier: f64,
    pub discrete_dividend_pv: f64,
    pub reasons: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
struct ScenarioDecisionThresholds {
    min_buy_return_pct: f64,
    min_buy_score: f64,
    min_buy_probability_target_pct: f64,
    min_buy_probability_breakeven_pct: f64,
    score_return_target_pct: f64,
}

impl Default for ScenarioDecisionThresholds {
    fn default() -> Self {
        Self {
            min_buy_return_pct: 8.0,
            min_buy_score: 70.0,
            min_buy_probability_target_pct: 10.0,
            min_buy_probability_breakeven_pct: 20.0,
            score_return_target_pct: 20.0,
        }
    }
}

impl ScenarioDecisionThresholds {
    fn from_env() -> Self {
        let default = Self::default();
        Self {
            min_buy_return_pct: read_env_f64("SCENARIO_MIN_BUY_RETURN_PCT")
                .unwrap_or(default.min_buy_return_pct),
            min_buy_score: read_env_f64("SCENARIO_MIN_BUY_SCORE")
                .unwrap_or(default.min_buy_score),
            min_buy_probability_target_pct: read_env_f64("SCENARIO_MIN_BUY_TARGET_PROB_PCT")
                .unwrap_or(default.min_buy_probability_target_pct),
            min_buy_probability_breakeven_pct: read_env_f64("SCENARIO_MIN_BUY_BREAKEVEN_PROB_PCT")
                .unwrap_or(default.min_buy_probability_breakeven_pct),
            score_return_target_pct: read_env_f64("SCENARIO_SCORE_RETURN_TARGET_PCT")
                .unwrap_or(default.score_return_target_pct)
                .max(0.01),
        }
    }
}

pub fn analyze_warrant_scenario(
    signals: &[OpportunitySignal],
    spot: f64,
    config: &ScenarioConfig,
    decision_config: &DecisionConfig,
    today: NaiveDate,
) -> Vec<ScenarioCandidate> {
    let mut candidates = signals
        .iter()
        .filter_map(|signal| analyze_signal(signal, spot, config, decision_config, today))
        .collect::<Vec<_>>();

    candidates.sort_by(|left, right| {
        decision_rank(left.decision)
            .cmp(&decision_rank(right.decision))
            .then_with(|| {
                right
                    .score
                    .partial_cmp(&left.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| {
                right
                    .net_return_pct
                    .partial_cmp(&left.net_return_pct)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| left.symbol.cmp(&right.symbol))
    });
    candidates
}

fn analyze_signal(
    signal: &OpportunitySignal,
    spot: f64,
    config: &ScenarioConfig,
    decision_config: &DecisionConfig,
    today: NaiveDate,
) -> Option<ScenarioCandidate> {
    if signal.product_family != "warrant" || signal.pricing_model != "warrant_intrinsic" {
        return None;
    }
    if signal.side != config.side {
        return None;
    }
    let maturity = parse_date(&signal.maturity)?;
    if maturity <= config.target_date {
        return None;
    }
    let maturity_before_preferred_window = config.min_maturity.is_some_and(|min| maturity < min);
    let maturity_after_preferred_window = config.max_maturity.is_some_and(|max| maturity > max);
    let entry_price = signal
        .ask_price
        .filter(|value| *value > 0.0)
        .or(signal.mid_price.filter(|value| *value > 0.0))
        .unwrap_or(signal.last_price);
    if entry_price <= 0.0 {
        return None;
    }
    let parity = signal.warrants_per_underlying?;
    if parity <= 0.0 || signal.fx_rate <= 0.0 {
        return None;
    }

    let kind = option_kind(&signal.side)?;
    let thresholds = ScenarioDecisionThresholds::from_env();
    let years_remaining = years_between(config.target_date, maturity)?;
    let years_to_target = years_between(today, config.target_date)?;
    let years_to_maturity = years_between(today, maturity)?;
    let volatility = signal
        .implied_volatility
        .or(signal.smile_median_iv)
        .unwrap_or(config.fallback_volatility)
        .clamp(0.0001, 5.0);
    let target_move_pct = if spot > 0.0 {
        (config.target_price - spot) / spot * 100.0
    } else {
        0.0
    };
    let dynamic_volatility_shift_points =
        config.vol_spot_slope_points_per_pct * target_move_pct;
    let exit_volatility =
        (volatility + dynamic_volatility_shift_points / 100.0).clamp(0.0001, 5.0);
    let fx_exit_rate = config.fx_target_rate.unwrap_or(signal.fx_rate);
    let fx_stressed_exit_rate = (config.fx_stress_pct > 0.0)
        .then_some((fx_exit_rate * (1.0 - config.fx_stress_pct / 100.0)).max(0.0001));
    let discrete_dividend_pv = present_value_dividends(
        config.target_date,
        maturity,
        config.risk_free_rate,
        &config.dividends,
    );
    let pricing_target = (config.target_price - discrete_dividend_pv).max(0.01);
    let raw_target_delta = raw_delta(
        kind,
        pricing_target,
        signal.strike,
        years_remaining,
        config.risk_free_rate,
        0.0,
        exit_volatility,
    )
    .unwrap_or(0.5);
    let effective_exit_spread_multiplier =
        dynamic_exit_spread_multiplier(config, raw_target_delta.abs());
    let projected_price = scenario_product_price(
        kind,
        pricing_target,
        signal.strike,
        years_remaining,
        config.risk_free_rate,
        scenario_dividend_yield(config),
        exit_volatility,
        parity,
        fx_exit_rate,
    )?;
    if projected_price <= 0.0 {
        return None;
    }
    let projected_bid_price =
        apply_exit_spread_penalty(projected_price, signal.spread_pct, effective_exit_spread_multiplier);
    let stressed_volatility =
        (exit_volatility + config.volatility_shock_points / 100.0).clamp(0.0001, 5.0);
    let stressed_projected_price = scenario_product_price(
        kind,
        pricing_target,
        signal.strike,
        years_remaining,
        config.risk_free_rate,
        scenario_dividend_yield(config),
        stressed_volatility,
        parity,
        fx_exit_rate,
    )
    .map(|price| apply_exit_spread_penalty(price, signal.spread_pct, effective_exit_spread_multiplier));
    let fx_stressed_projected_price = fx_stressed_exit_rate.and_then(|fx_rate| {
        scenario_product_price(
            kind,
            pricing_target,
            signal.strike,
            years_remaining,
            config.risk_free_rate,
            scenario_dividend_yield(config),
            stressed_volatility,
            parity,
            fx_rate,
        )
        .map(|price| apply_exit_spread_penalty(price, signal.spread_pct, effective_exit_spread_multiplier))
    });

    let gross_return_pct = (projected_bid_price - entry_price) / entry_price * 100.0;
    let fee_result = net_return_after_fees(
        entry_price,
        projected_bid_price,
        decision_config.fee_order_notional,
        decision_config.fee_buy_fixed,
        decision_config.fee_buy_pct,
        decision_config.fee_sell_fixed,
        decision_config.fee_sell_pct,
        decision_config.fee_deposit_fixed,
        decision_config.fee_deposit_pct,
    )?;
    let stressed_net_return_pct = stressed_projected_price.and_then(|price| {
        net_return_after_fees(
            entry_price,
            price,
            decision_config.fee_order_notional,
            decision_config.fee_buy_fixed,
            decision_config.fee_buy_pct,
            decision_config.fee_sell_fixed,
            decision_config.fee_sell_pct,
            decision_config.fee_deposit_fixed,
            decision_config.fee_deposit_pct,
        )
        .map(|result| result.net_return_pct)
    });
    let fx_stressed_net_return_pct = fx_stressed_projected_price.and_then(|price| {
        net_return_after_fees(
            entry_price,
            price,
            decision_config.fee_order_notional,
            decision_config.fee_buy_fixed,
            decision_config.fee_buy_pct,
            decision_config.fee_sell_fixed,
            decision_config.fee_sell_pct,
            decision_config.fee_deposit_fixed,
            decision_config.fee_deposit_pct,
        )
        .map(|result| result.net_return_pct)
    });
    let breakeven_underlying =
        solve_breakeven_underlying(kind, signal, years_remaining, exit_volatility, config, decision_config);
    let breakeven_distance_pct =
        breakeven_underlying.map(|value| directional_distance_pct(kind, spot, value));
    let theta_horizon_pct = theta_horizon_pct(
        kind,
        signal,
        spot,
        entry_price,
        volatility,
        today,
        config.target_date,
        maturity,
        config,
    );
    let probability = scenario_probabilities(
        kind,
        spot,
        config.target_price,
        breakeven_underlying,
        years_to_target,
        config.risk_free_rate - config.dividend_yield,
        config.real_world_drift,
        volatility,
    );
    let risk_neutral_ev_pct = risk_neutral_expected_value_pct(
        kind,
        spot,
        signal.strike,
        years_to_maturity,
        years_to_target,
        config.risk_free_rate,
        config.dividend_yield,
        volatility,
        parity,
        signal.fx_rate,
        entry_price,
        decision_config,
    );
    let binary_stats = binary_scenario_stats(
        probability.probability_target_pct,
        fee_result.net_return_pct,
        -decision_config.max_loss_pct_per_trade.abs(),
    );
    let greeks = scenario_greeks(
        kind,
        spot,
        signal.strike,
        years_to_maturity,
        config.risk_free_rate,
        config.dividend_yield,
        volatility,
        parity,
        signal.fx_rate,
    );
    let target_greeks = scenario_greeks(
        kind,
        pricing_target,
        signal.strike,
        years_remaining,
        config.risk_free_rate,
        scenario_dividend_yield(config),
        exit_volatility,
        parity,
        fx_exit_rate,
    );

    let mut reasons = Vec::new();
    let mut warnings = scenario_data_warnings(signal, config);
    if signal.execution_status != "executable_bid_ask" || signal.ask_price.is_none() {
        reasons.push("entry_price_not_executable".to_string());
    }
    if maturity_before_preferred_window {
        reasons.push("maturity_before_preferred_window".to_string());
    }
    if maturity_after_preferred_window {
        reasons.push("maturity_after_preferred_window".to_string());
    }
    if fee_result.net_return_pct <= 0.0 {
        reasons.push("scenario_return_negative".to_string());
    }
    if signal.data_quality_score < decision_config.min_data_quality_score {
        reasons.push("data_quality_too_low".to_string());
    }
    if signal.spread_pct.unwrap_or(f64::INFINITY) > decision_config.max_spread_pct {
        reasons.push("spread_too_wide".to_string());
    }
    if breakeven_distance_pct.is_some_and(|distance| {
        let target_distance = directional_distance_pct(kind, spot, config.target_price);
        distance > target_distance
    }) {
        reasons.push("breakeven_beyond_target".to_string());
    }
    if fee_result.net_return_pct > 0.0
        && fee_result.net_return_pct < thresholds.min_buy_return_pct
    {
        reasons.push("scenario_return_below_buy_threshold".to_string());
    }
    if probability.probability_target_pct.is_some_and(|probability| {
        probability < thresholds.min_buy_probability_target_pct
    }) {
        reasons.push("target_probability_too_low".to_string());
    }
    if probability.probability_breakeven_pct.is_some_and(|probability| {
        probability < thresholds.min_buy_probability_breakeven_pct
    }) {
        reasons.push("breakeven_probability_too_low".to_string());
    }
    if stressed_net_return_pct.is_some_and(|value| value <= 0.0) {
        reasons.push("volatility_crush_erases_return".to_string());
    }
    if fx_stressed_net_return_pct.is_some_and(|value| value <= 0.0) {
        reasons.push("fx_stress_erases_return".to_string());
    }
    if discrete_dividend_pv > 0.0 {
        warnings.push("discrete_dividends_applied".to_string());
    }
    if effective_exit_spread_multiplier > config.exit_spread_multiplier * 1.05 {
        warnings.push("dynamic_exit_spread_penalty".to_string());
    }

    let score = scenario_score(
        fee_result.net_return_pct,
        signal.data_quality_score,
        signal.spread_pct,
        signal.smile_gap_vol_points,
        maturity_fit_score(config.target_date, maturity),
        &thresholds,
    );
    let decision = scenario_decision(fee_result.net_return_pct, score, &reasons, &thresholds);

    Some(ScenarioCandidate {
        decision,
        score,
        symbol: signal.symbol.clone(),
        side: signal.side.clone(),
        product_type: signal.product_type.clone(),
        url: signal.web_url.clone(),
        strike: signal.strike,
        maturity,
        entry_price,
        projected_price,
        projected_bid_price,
        stressed_projected_price,
        fx_stressed_projected_price,
        net_return_pct: fee_result.net_return_pct,
        stressed_net_return_pct,
        fx_stressed_net_return_pct,
        gross_return_pct,
        fee_drag_pct: fee_result.fee_drag_pct,
        breakeven_underlying,
        breakeven_distance_pct,
        implied_volatility: signal.implied_volatility,
        volatility_used: volatility,
        exit_volatility,
        dynamic_volatility_shift_points,
        volatility_shock_points: config.volatility_shock_points,
        theta_horizon_pct,
        spread_pct: signal.spread_pct,
        probability_target_pct: probability.probability_target_pct,
        probability_breakeven_pct: probability.probability_breakeven_pct,
        terminal_probability_target_pct: probability.terminal_probability_target_pct,
        terminal_probability_breakeven_pct: probability.terminal_probability_breakeven_pct,
        target_zscore: probability.target_zscore,
        breakeven_zscore: probability.breakeven_zscore,
        expected_value_pct: risk_neutral_ev_pct,
        sharpe_like: binary_stats.sharpe_like,
        kelly_fraction_pct: binary_stats.kelly_fraction_pct,
        delta: greeks.as_ref().map(|value| value.delta),
        target_delta: target_greeks.as_ref().map(|value| value.delta),
        gamma: greeks.as_ref().map(|value| value.gamma),
        target_gamma: target_greeks.as_ref().map(|value| value.gamma),
        vega_per_vol_point: greeks.as_ref().map(|value| value.vega_per_vol_point),
        target_vega_per_vol_point: target_greeks.as_ref().map(|value| value.vega_per_vol_point),
        theta_per_day: greeks.as_ref().map(|value| value.theta_per_day),
        target_theta_per_day: target_greeks.as_ref().map(|value| value.theta_per_day),
        rho_per_rate_point: greeks.as_ref().map(|value| value.rho_per_rate_point),
        d1: greeks.as_ref().map(|value| value.d1),
        d2: greeks.as_ref().map(|value| value.d2),
        data_quality_score: signal.data_quality_score,
        liquidity_score: signal.liquidity_score,
        parity,
        fx_rate: signal.fx_rate,
        fx_exit_rate,
        fx_stressed_exit_rate,
        effective_exit_spread_multiplier,
        discrete_dividend_pv,
        reasons,
        warnings,
    })
}

fn scenario_product_price(
    kind: OptionKind,
    target_price: f64,
    strike: f64,
    years_remaining: f64,
    risk_free_rate: f64,
    dividend_yield: f64,
    volatility: f64,
    parity: f64,
    fx_rate: f64,
) -> Option<f64> {
    let price_per_underlying = black_scholes_price(BlackScholesInput {
        kind,
        spot: target_price,
        strike,
        years_to_maturity: years_remaining,
        risk_free_rate,
        dividend_yield,
        volatility,
    })?;
    Some(price_per_underlying / parity * fx_rate)
}

fn apply_exit_spread_penalty(price: f64, spread_pct: Option<f64>, multiplier: f64) -> f64 {
    let spread = spread_pct.unwrap_or(0.0).max(0.0);
    let penalty = spread * multiplier.max(0.0) / 100.0;
    (price * (1.0 - penalty)).max(0.0)
}

fn dynamic_exit_spread_multiplier(config: &ScenarioConfig, raw_delta_abs: f64) -> f64 {
    let edge_penalty = ((raw_delta_abs - 0.75) / 0.25)
        .max((0.20 - raw_delta_abs) / 0.20)
        .max(0.0)
        .clamp(0.0, 2.0);
    config.exit_spread_multiplier
        * (1.0 + edge_penalty * config.exit_spread_delta_penalty.max(0.0))
}

fn scenario_dividend_yield(config: &ScenarioConfig) -> f64 {
    if config.dividends.is_empty() {
        config.dividend_yield
    } else {
        0.0
    }
}

fn present_value_dividends(
    from_date: NaiveDate,
    to_date: NaiveDate,
    risk_free_rate: f64,
    dividends: &[DiscreteDividend],
) -> f64 {
    dividends
        .iter()
        .filter(|dividend| {
            dividend.amount > 0.0
                && dividend.ex_date > from_date
                && dividend.ex_date <= to_date
        })
        .filter_map(|dividend| {
            years_between(from_date, dividend.ex_date)
                .map(|years| dividend.amount * (-risk_free_rate * years).exp())
        })
        .sum()
}

fn raw_delta(
    kind: OptionKind,
    spot: f64,
    strike: f64,
    years_to_maturity: f64,
    risk_free_rate: f64,
    dividend_yield: f64,
    volatility: f64,
) -> Option<f64> {
    black_scholes_greeks(BlackScholesInput {
        kind,
        spot,
        strike,
        years_to_maturity,
        risk_free_rate,
        dividend_yield,
        volatility,
    })
    .map(|greeks| greeks.delta)
}

fn scenario_data_warnings(signal: &OpportunitySignal, config: &ScenarioConfig) -> Vec<String> {
    let mut warnings = Vec::new();
    if config.stale_pricing_guard
        && signal.strike_currency.eq_ignore_ascii_case("USD")
        && config
            .paris_hour
            .is_some_and(|hour| (9..15).contains(&hour))
    {
        warnings.push("possible_us_premarket_spot_mismatch".to_string());
    }
    warnings
}

#[derive(Debug, Clone, Copy, Default)]
struct ScenarioProbabilities {
    probability_target_pct: Option<f64>,
    probability_breakeven_pct: Option<f64>,
    terminal_probability_target_pct: Option<f64>,
    terminal_probability_breakeven_pct: Option<f64>,
    target_zscore: Option<f64>,
    breakeven_zscore: Option<f64>,
}

fn scenario_probabilities(
    kind: OptionKind,
    spot: f64,
    target_price: f64,
    breakeven_underlying: Option<f64>,
    years_to_target: f64,
    risk_neutral_drift: f64,
    real_world_drift: f64,
    volatility: f64,
) -> ScenarioProbabilities {
    let terminal_target = lognormal_probability(
        kind,
        spot,
        target_price,
        years_to_target,
        risk_neutral_drift,
        volatility,
    )
        .map(|(probability, zscore)| (probability * 100.0, zscore));
    let breakeven = breakeven_underlying.and_then(|level| {
        lognormal_probability(kind, spot, level, years_to_target, risk_neutral_drift, volatility)
            .map(|(probability, zscore)| (probability * 100.0, zscore))
    });
    let touch_target = first_touch_probability(
        kind,
        spot,
        target_price,
        years_to_target,
        real_world_drift,
        volatility,
    )
    .map(|probability| probability * 100.0);
    let touch_breakeven = breakeven_underlying.and_then(|level| {
        first_touch_probability(kind, spot, level, years_to_target, real_world_drift, volatility)
            .map(|probability| probability * 100.0)
    });

    ScenarioProbabilities {
        probability_target_pct: touch_target,
        probability_breakeven_pct: touch_breakeven,
        terminal_probability_target_pct: terminal_target.map(|value| value.0),
        terminal_probability_breakeven_pct: breakeven.map(|value| value.0),
        target_zscore: terminal_target.map(|value| value.1),
        breakeven_zscore: breakeven.map(|value| value.1),
    }
}

#[allow(clippy::too_many_arguments)]
fn risk_neutral_expected_value_pct(
    kind: OptionKind,
    spot: f64,
    strike: f64,
    years_to_maturity: f64,
    years_to_target: f64,
    risk_free_rate: f64,
    dividend_yield: f64,
    volatility: f64,
    parity: f64,
    fx_rate: f64,
    entry_price: f64,
    decision_config: &DecisionConfig,
) -> Option<f64> {
    let fair_now = scenario_product_price(
        kind,
        spot,
        strike,
        years_to_maturity,
        risk_free_rate,
        dividend_yield,
        volatility,
        parity,
        fx_rate,
    )?;
    let expected_exit_price = fair_now * (risk_free_rate * years_to_target).exp();
    net_return_after_fees(
        entry_price,
        expected_exit_price,
        decision_config.fee_order_notional,
        decision_config.fee_buy_fixed,
        decision_config.fee_buy_pct,
        decision_config.fee_sell_fixed,
        decision_config.fee_sell_pct,
        decision_config.fee_deposit_fixed,
        decision_config.fee_deposit_pct,
    )
    .map(|result| result.net_return_pct)
}

#[derive(Debug, Clone, Copy, Default)]
struct BinaryScenarioStats {
    sharpe_like: Option<f64>,
    kelly_fraction_pct: Option<f64>,
}

fn binary_scenario_stats(
    probability_win_pct: Option<f64>,
    gain_pct: f64,
    loss_pct: f64,
) -> BinaryScenarioStats {
    let Some(probability_win_pct) = probability_win_pct else {
        return BinaryScenarioStats::default();
    };
    if gain_pct <= 0.0 || loss_pct >= 0.0 {
        return BinaryScenarioStats::default();
    }

    let p = (probability_win_pct / 100.0).clamp(0.0, 1.0);
    let q = 1.0 - p;
    let gain = gain_pct / 100.0;
    let loss = loss_pct / 100.0;
    let expected = p * gain + q * loss;
    let variance = p * (gain - expected).powi(2) + q * (loss - expected).powi(2);
    let sharpe_like = (variance > 0.0).then_some(expected / variance.sqrt());
    let b = gain / loss.abs();
    let kelly_fraction_pct = (b > 0.0)
        .then_some(((b * p - q) / b).clamp(0.0, 1.0) * 100.0);

    BinaryScenarioStats {
        sharpe_like,
        kelly_fraction_pct,
    }
}

#[allow(clippy::too_many_arguments)]
fn scenario_greeks(
    kind: OptionKind,
    spot: f64,
    strike: f64,
    years_to_maturity: f64,
    risk_free_rate: f64,
    dividend_yield: f64,
    volatility: f64,
    parity: f64,
    fx_rate: f64,
) -> Option<crate::indicators::options::BlackScholesGreeks> {
    let raw = black_scholes_greeks(BlackScholesInput {
        kind,
        spot,
        strike,
        years_to_maturity,
        risk_free_rate,
        dividend_yield,
        volatility,
    })?;
    let scale = fx_rate / parity;
    Some(crate::indicators::options::BlackScholesGreeks {
        delta: raw.delta * scale,
        gamma: raw.gamma * scale,
        vega_per_vol_point: raw.vega_per_vol_point * scale,
        theta_per_day: raw.theta_per_day * scale,
        rho_per_rate_point: raw.rho_per_rate_point * scale,
        d1: raw.d1,
        d2: raw.d2,
    })
}

struct FeeResult {
    net_return_pct: f64,
    fee_drag_pct: f64,
}

fn net_return_after_fees(
    entry_price: f64,
    exit_price: f64,
    order_notional: f64,
    buy_fixed: f64,
    buy_pct: f64,
    sell_fixed: f64,
    sell_pct: f64,
    deposit_fixed: f64,
    deposit_pct: f64,
) -> Option<FeeResult> {
    if entry_price <= 0.0 || exit_price < 0.0 || order_notional <= 0.0 {
        return None;
    }
    let buy_fee = buy_fixed + order_notional * buy_pct / 100.0;
    let deposit_fee = deposit_fixed + order_notional * deposit_pct / 100.0;
    let exit_notional = order_notional * exit_price / entry_price;
    let sell_fee = sell_fixed + exit_notional * sell_pct / 100.0;
    let cost_basis = order_notional + buy_fee + deposit_fee;
    let net_return_pct = ((exit_notional - sell_fee) - cost_basis) / cost_basis * 100.0;
    let fee_drag_pct = (buy_fee + deposit_fee + sell_fee) / order_notional * 100.0;
    Some(FeeResult {
        net_return_pct,
        fee_drag_pct,
    })
}

fn solve_breakeven_underlying(
    kind: OptionKind,
    signal: &OpportunitySignal,
    years_remaining: f64,
    volatility: f64,
    config: &ScenarioConfig,
    decision_config: &DecisionConfig,
) -> Option<f64> {
    let mut low = 0.01;
    let mut high = (signal.strike.max(config.target_price) * 2.5).max(1.0);
    if kind == OptionKind::Put {
        low = 0.01;
        high = signal.strike.max(config.target_price).max(1.0);
    }

    for _ in 0..80 {
        let mid = (low + high) / 2.0;
        let dividend_pv = present_value_dividends(
            config.target_date,
            parse_date(&signal.maturity)?,
            config.risk_free_rate,
            &config.dividends,
        );
        let pricing_mid = (mid - dividend_pv).max(0.01);
        let raw_delta_abs = raw_delta(
            kind,
            pricing_mid,
            signal.strike,
            years_remaining,
            config.risk_free_rate,
            scenario_dividend_yield(config),
            volatility,
        )
        .unwrap_or(0.5)
        .abs();
        let spread_multiplier = dynamic_exit_spread_multiplier(config, raw_delta_abs);
        let price = scenario_product_price(
            kind,
            pricing_mid,
            signal.strike,
            years_remaining,
            config.risk_free_rate,
            scenario_dividend_yield(config),
            volatility,
            signal.warrants_per_underlying?,
            config.fx_target_rate.unwrap_or(signal.fx_rate),
        )?;
        let price = apply_exit_spread_penalty(price, signal.spread_pct, spread_multiplier);
        let net = net_return_after_fees(
            signal.ask_price?,
            price,
            decision_config.fee_order_notional,
            decision_config.fee_buy_fixed,
            decision_config.fee_buy_pct,
            decision_config.fee_sell_fixed,
            decision_config.fee_sell_pct,
            decision_config.fee_deposit_fixed,
            decision_config.fee_deposit_pct,
        )?
        .net_return_pct;

        match kind {
            OptionKind::Call if net >= 0.0 => high = mid,
            OptionKind::Call => low = mid,
            OptionKind::Put if net >= 0.0 => low = mid,
            OptionKind::Put => high = mid,
        }
    }

    Some((low + high) / 2.0)
}

#[allow(clippy::too_many_arguments)]
fn theta_horizon_pct(
    kind: OptionKind,
    signal: &OpportunitySignal,
    spot: f64,
    entry_price: f64,
    volatility: f64,
    today: NaiveDate,
    target_date: NaiveDate,
    maturity: NaiveDate,
    config: &ScenarioConfig,
) -> Option<f64> {
    if entry_price <= 0.0 {
        return None;
    }
    let years_to_maturity = years_between(today, maturity)?;
    let years_remaining = years_between(target_date, maturity)?;
    let parity = signal.warrants_per_underlying?;
    let model_now = scenario_product_price(
        kind,
        spot,
        signal.strike,
        years_to_maturity,
        config.risk_free_rate,
        config.dividend_yield,
        volatility,
        parity,
        signal.fx_rate,
    )?;
    let model_at_target_same_spot = scenario_product_price(
        kind,
        spot,
        signal.strike,
        years_remaining,
        config.risk_free_rate,
        config.dividend_yield,
        volatility,
        parity,
        signal.fx_rate,
    )?;
    let decay = (model_now - model_at_target_same_spot).max(0.0);
    Some(decay / entry_price * 100.0)
}

fn scenario_score(
    net_return_pct: f64,
    data_quality_score: f64,
    spread_pct: Option<f64>,
    smile_gap_vol_points: Option<f64>,
    maturity_fit_score: f64,
    thresholds: &ScenarioDecisionThresholds,
) -> f64 {
    let return_score =
        (net_return_pct.max(0.0) / thresholds.score_return_target_pct * 100.0).clamp(0.0, 100.0);
    let spread_score = spread_pct
        .map(|spread| 100.0 - (spread / 5.0 * 100.0).clamp(0.0, 100.0))
        .unwrap_or(0.0);
    let iv_score = smile_gap_vol_points
        .map(|gap| (50.0 - gap * 8.0).clamp(0.0, 100.0))
        .unwrap_or(50.0);

    return_score * 0.35
        + data_quality_score * 0.25
        + spread_score * 0.15
        + iv_score * 0.10
        + maturity_fit_score * 0.15
}

fn scenario_decision(
    net_return_pct: f64,
    score: f64,
    reasons: &[String],
    thresholds: &ScenarioDecisionThresholds,
) -> &'static str {
    if reasons.is_empty()
        && net_return_pct >= thresholds.min_buy_return_pct
        && score >= thresholds.min_buy_score
    {
        "BUY"
    } else if net_return_pct > 0.0 && reasons.len() <= 1 {
        "WATCH"
    } else {
        "AVOID"
    }
}

fn read_env_f64(name: &str) -> Option<f64> {
    std::env::var(name)
        .ok()
        .as_deref()
        .and_then(|value| value.parse().ok())
}

fn maturity_fit_score(target_date: NaiveDate, maturity: NaiveDate) -> f64 {
    let days_after_target = (maturity - target_date).num_days();
    if days_after_target < 0 {
        return 0.0;
    }
    let ideal = 75.0;
    let distance = (days_after_target as f64 - ideal).abs();
    (100.0 - distance / ideal * 100.0).clamp(0.0, 100.0)
}

fn directional_distance_pct(kind: OptionKind, spot: f64, target: f64) -> f64 {
    match kind {
        OptionKind::Call => (target - spot) / spot * 100.0,
        OptionKind::Put => (spot - target) / spot * 100.0,
    }
}

fn years_between(start: NaiveDate, end: NaiveDate) -> Option<f64> {
    let days = (end - start).num_days();
    (days > 0).then_some(days as f64 / 365.0)
}

fn parse_date(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()
}

fn option_kind(side: &str) -> Option<OptionKind> {
    match side {
        "call" => Some(OptionKind::Call),
        "put" => Some(OptionKind::Put),
        _ => None,
    }
}

fn decision_rank(decision: &str) -> u8 {
    match decision {
        "BUY" => 0,
        "WATCH" => 1,
        _ => 2,
    }
}

pub fn default_max_maturity_for_early_next_year(target_date: NaiveDate) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(target_date.year() + 1, 3, 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indicators::warrant::ValuationSide;

    #[test]
    fn call_scenario_projects_positive_return_when_target_is_above_strike() {
        let signal = signal();
        let scenario = ScenarioConfig {
            target_price: 1800.0,
            target_date: NaiveDate::from_ymd_opt(2026, 10, 31).unwrap(),
            min_maturity: Some(NaiveDate::from_ymd_opt(2027, 1, 1).unwrap()),
            max_maturity: Some(NaiveDate::from_ymd_opt(2027, 3, 31).unwrap()),
            side: "call".to_string(),
            risk_free_rate: 0.045,
            dividend_yield: 0.005,
            fallback_volatility: 0.35,
            volatility_shock_points: -5.0,
            real_world_drift: 0.08,
            fx_target_rate: None,
            fx_stress_pct: 0.0,
            vol_spot_slope_points_per_pct: -0.50,
            exit_spread_multiplier: 1.5,
            exit_spread_delta_penalty: 0.75,
            stale_pricing_guard: false,
            paris_hour: None,
            dividends: Vec::new(),
        };

        let candidates = analyze_warrant_scenario(
            &[signal],
            1533.5,
            &scenario,
            &DecisionConfig::default(),
            NaiveDate::from_ymd_opt(2026, 5, 18).unwrap(),
        );

        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].projected_price > candidates[0].entry_price);
        assert!(candidates[0].net_return_pct > 0.0);
        assert!(candidates[0].breakeven_underlying.is_some());
    }

    #[test]
    fn scenario_adds_probabilities_expected_value_and_scaled_greeks() {
        let candidates = analyze_warrant_scenario(
            &[signal()],
            1533.5,
            &scenario(),
            &DecisionConfig::default(),
            NaiveDate::from_ymd_opt(2026, 5, 18).unwrap(),
        );
        let candidate = &candidates[0];

        assert!(candidate.probability_target_pct.unwrap() > 0.0);
        assert!(candidate.probability_breakeven_pct.unwrap() > 0.0);
        assert!(candidate.target_zscore.unwrap().is_finite());
        assert!(candidate.breakeven_zscore.unwrap().is_finite());
        assert!(candidate.expected_value_pct.unwrap().is_finite());
        assert!(candidate.sharpe_like.unwrap().is_finite());
        assert!(candidate.kelly_fraction_pct.unwrap() >= 0.0);
        assert!(candidate.delta.unwrap() > 0.0);
        assert!(candidate.gamma.unwrap() > 0.0);
        assert!(candidate.vega_per_vol_point.unwrap() > 0.0);
        assert!(candidate.theta_per_day.unwrap() < 0.0);
        assert!(candidate.theta_horizon_pct.unwrap() >= 0.0);
        assert!(candidate.stressed_projected_price.unwrap() < candidate.projected_price);
        assert!(candidate.stressed_net_return_pct.unwrap() < candidate.net_return_pct);
        assert!(candidate.d1.unwrap() > candidate.d2.unwrap());
    }

    #[test]
    fn volatility_crush_lowers_a_nominally_positive_scenario() {
        let scenario = ScenarioConfig {
            target_price: 1800.0,
            volatility_shock_points: -34.0,
            ..scenario()
        };

        let candidates = analyze_warrant_scenario(
            &[signal()],
            1533.5,
            &scenario,
            &DecisionConfig::default(),
            NaiveDate::from_ymd_opt(2026, 5, 18).unwrap(),
        );

        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].net_return_pct > 0.0);
        assert!(candidates[0].stressed_net_return_pct.unwrap() < candidates[0].net_return_pct);
    }

    #[test]
    fn first_touch_probability_is_used_for_decision_probability() {
        let candidates = analyze_warrant_scenario(
            &[signal()],
            1533.5,
            &scenario(),
            &DecisionConfig::default(),
            NaiveDate::from_ymd_opt(2026, 5, 18).unwrap(),
        );
        let candidate = &candidates[0];

        assert!(
            candidate.probability_target_pct.unwrap()
                > candidate.terminal_probability_target_pct.unwrap(),
            "touch={:?}, terminal={:?}",
            candidate.probability_target_pct,
            candidate.terminal_probability_target_pct
        );
    }

    #[test]
    fn dynamic_iv_and_exit_spread_reduce_exit_value() {
        let mut scenario = scenario();
        scenario.target_price = 1900.0;
        scenario.vol_spot_slope_points_per_pct = -1.0;
        scenario.exit_spread_multiplier = 2.0;
        let candidates = analyze_warrant_scenario(
            &[signal()],
            1533.5,
            &scenario,
            &DecisionConfig::default(),
            NaiveDate::from_ymd_opt(2026, 5, 18).unwrap(),
        );
        let candidate = &candidates[0];

        assert!(candidate.exit_volatility < candidate.volatility_used);
        assert!(candidate.projected_bid_price < candidate.projected_price);
    }

    #[test]
    fn discrete_dividend_reduces_call_projection_and_is_reported() {
        let mut with_dividend = scenario();
        with_dividend.dividends = vec![DiscreteDividend {
            ex_date: NaiveDate::from_ymd_opt(2026, 12, 1).unwrap(),
            amount: 25.0,
        }];
        let without_dividend = ScenarioConfig {
            dividends: Vec::new(),
            ..with_dividend.clone()
        };

        let with_candidates = analyze_warrant_scenario(
            &[signal()],
            1533.5,
            &with_dividend,
            &DecisionConfig::default(),
            NaiveDate::from_ymd_opt(2026, 5, 18).unwrap(),
        );
        let without_candidates = analyze_warrant_scenario(
            &[signal()],
            1533.5,
            &without_dividend,
            &DecisionConfig::default(),
            NaiveDate::from_ymd_opt(2026, 5, 18).unwrap(),
        );

        assert!(with_candidates[0].discrete_dividend_pv > 0.0);
        assert!(with_candidates[0].projected_price < without_candidates[0].projected_price);
        assert!(with_candidates[0]
            .warnings
            .contains(&"discrete_dividends_applied".to_string()));
    }

    #[test]
    fn projected_greeks_are_available_at_target() {
        let candidates = analyze_warrant_scenario(
            &[signal()],
            1533.5,
            &scenario(),
            &DecisionConfig::default(),
            NaiveDate::from_ymd_opt(2026, 5, 18).unwrap(),
        );
        let candidate = &candidates[0];

        assert!(candidate.target_delta.is_some());
        assert!(candidate.target_gamma.is_some());
        assert!(candidate.target_vega_per_vol_point.is_some());
        assert!(candidate.target_theta_per_day.is_some());
    }

    #[test]
    fn stale_pricing_guard_warns_for_us_underlying_before_us_open() {
        let mut config = scenario();
        config.stale_pricing_guard = true;
        config.paris_hour = Some(10);
        let mut us_signal = signal();
        us_signal.strike_currency = "USD".to_string();

        let candidates = analyze_warrant_scenario(
            &[us_signal],
            1533.5,
            &config,
            &DecisionConfig::default(),
            NaiveDate::from_ymd_opt(2026, 5, 18).unwrap(),
        );

        assert!(candidates[0]
            .warnings
            .contains(&"possible_us_premarket_spot_mismatch".to_string()));
    }

    #[test]
    fn adverse_fx_stress_can_block_nominal_gain() {
        let mut scenario = scenario();
        scenario.fx_stress_pct = 60.0;
        let candidates = analyze_warrant_scenario(
            &[signal()],
            1533.5,
            &scenario,
            &DecisionConfig::default(),
            NaiveDate::from_ymd_opt(2026, 5, 18).unwrap(),
        );
        let candidate = &candidates[0];

        assert!(candidate.fx_stressed_net_return_pct.unwrap() < candidate.net_return_pct);
        assert!(candidate
            .reasons
            .contains(&"fx_stress_erases_return".to_string()));
    }

    #[test]
    fn smaller_scenario_return_can_still_be_buy() {
        let thresholds = ScenarioDecisionThresholds::default();
        let score = scenario_score(
            10.0,
            100.0,
            Some(0.5),
            Some(0.0),
            100.0,
            &thresholds,
        );
        let reasons = Vec::new();

        assert!(score >= thresholds.min_buy_score);
        assert_eq!(scenario_decision(10.0, score, &reasons, &thresholds), "BUY");
    }

    #[test]
    fn positive_return_below_buy_threshold_is_watch() {
        let thresholds = ScenarioDecisionThresholds::default();
        let reasons = vec!["scenario_return_below_buy_threshold".to_string()];

        assert_eq!(scenario_decision(5.0, 90.0, &reasons, &thresholds), "WATCH");
    }

    #[test]
    fn maturity_before_target_is_rejected() {
        let mut signal = signal();
        signal.maturity = "2026-09-18".to_string();
        let scenario = scenario();

        let candidates = analyze_warrant_scenario(
            &[signal],
            1533.5,
            &scenario,
            &DecisionConfig::default(),
            NaiveDate::from_ymd_opt(2026, 5, 18).unwrap(),
        );

        assert!(candidates.is_empty());
    }

    #[test]
    fn min_and_max_maturity_window_is_reported_without_hiding_candidates() {
        let mut too_early = signal();
        too_early.maturity = "2026-12-18".to_string();
        let mut too_late = signal();
        too_late.maturity = "2027-06-18".to_string();

        let candidates = analyze_warrant_scenario(
            &[too_early, too_late],
            1533.5,
            &scenario(),
            &DecisionConfig::default(),
            NaiveDate::from_ymd_opt(2026, 5, 18).unwrap(),
        );

        assert_eq!(candidates.len(), 2);
        assert!(candidates.iter().any(|candidate| candidate
            .reasons
            .contains(&"maturity_before_preferred_window".to_string())));
        assert!(candidates.iter().any(|candidate| candidate
            .reasons
            .contains(&"maturity_after_preferred_window".to_string())));
    }

    #[test]
    fn side_filter_keeps_only_the_intended_direction() {
        let mut put = signal();
        put.symbol = "RMSPUT".to_string();
        put.side = "put".to_string();
        put.product_type = "Warrant Put".to_string();
        put.strike = 1400.0;
        let scenario = ScenarioConfig {
            side: "put".to_string(),
            target_price: 1300.0,
            ..scenario()
        };

        let candidates = analyze_warrant_scenario(
            &[signal(), put],
            1533.5,
            &scenario,
            &DecisionConfig::default(),
            NaiveDate::from_ymd_opt(2026, 5, 18).unwrap(),
        );

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].symbol, "RMSPUT");
        assert_eq!(candidates[0].side, "put");
    }

    #[test]
    fn non_warrant_products_are_rejected() {
        let mut turbo = signal();
        turbo.product_family = "turbo".to_string();
        turbo.pricing_model = "financing_level".to_string();

        let candidates = analyze_warrant_scenario(
            &[turbo],
            1533.5,
            &scenario(),
            &DecisionConfig::default(),
            NaiveDate::from_ymd_opt(2026, 5, 18).unwrap(),
        );

        assert!(candidates.is_empty());
    }

    #[test]
    fn broker_fees_reduce_projected_return() {
        let mut config = DecisionConfig {
            broker_fee_profile: "unit".to_string(),
            fee_order_notional: 1000.0,
            fee_buy_fixed: 1.90,
            fee_sell_fixed: 1.90,
            ..DecisionConfig::default()
        };
        let with_fees = analyze_warrant_scenario(
            &[signal()],
            1533.5,
            &scenario(),
            &config,
            NaiveDate::from_ymd_opt(2026, 5, 18).unwrap(),
        );
        config.fee_buy_fixed = 0.0;
        config.fee_sell_fixed = 0.0;
        let without_fees = analyze_warrant_scenario(
            &[signal()],
            1533.5,
            &scenario(),
            &config,
            NaiveDate::from_ymd_opt(2026, 5, 18).unwrap(),
        );

        assert_eq!(with_fees.len(), 1);
        assert_eq!(without_fees.len(), 1);
        assert!(with_fees[0].net_return_pct < without_fees[0].net_return_pct);
        assert!(with_fees[0].fee_drag_pct > 0.0);
    }

    #[test]
    fn low_trade_flow_does_not_reject_scenario_candidate() {
        let mut low_liq = signal();
        low_liq.liquidity_score = 20.0;

        let candidates = analyze_warrant_scenario(
            &[low_liq],
            1533.5,
            &scenario(),
            &DecisionConfig::default(),
            NaiveDate::from_ymd_opt(2026, 5, 18).unwrap(),
        );

        assert_eq!(candidates.len(), 1);
        assert!(!candidates[0]
            .reasons
            .contains(&"liquidity_too_low".to_string()));
        assert_ne!(candidates[0].decision, "AVOID");
    }

    #[test]
    fn non_executable_scenario_price_is_kept_but_avoided() {
        let mut unverified = signal();
        unverified.price_source = "last_unverified".to_string();
        unverified.execution_status = "last_unverified".to_string();
        unverified.ask_price = None;
        unverified.bid_price = None;
        unverified.mid_price = None;
        unverified.spread_pct = None;
        unverified.last_price = 1.15;

        let candidates = analyze_warrant_scenario(
            &[unverified],
            1533.5,
            &scenario(),
            &DecisionConfig::default(),
            NaiveDate::from_ymd_opt(2026, 5, 18).unwrap(),
        );

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].entry_price, 1.15);
        assert!(candidates[0]
            .reasons
            .contains(&"entry_price_not_executable".to_string()));
        assert_eq!(candidates[0].decision, "AVOID");
    }

    #[test]
    fn breakeven_beyond_target_is_reported() {
        let mut expensive = signal();
        expensive.ask_price = Some(10.0);
        expensive.last_price = 10.0;
        let scenario = ScenarioConfig {
            target_price: 1710.0,
            ..scenario()
        };

        let candidates = analyze_warrant_scenario(
            &[expensive],
            1533.5,
            &scenario,
            &DecisionConfig::default(),
            NaiveDate::from_ymd_opt(2026, 5, 18).unwrap(),
        );

        assert_eq!(candidates.len(), 1);
        assert!(candidates[0]
            .reasons
            .contains(&"breakeven_beyond_target".to_string()));
        assert_eq!(candidates[0].decision, "AVOID");
    }

    #[test]
    fn candidates_are_sorted_by_decision_then_score() {
        let mut good = signal();
        good.symbol = "GOOD".to_string();
        let mut weak = signal();
        weak.symbol = "WEAK".to_string();
        weak.data_quality_score = 20.0;

        let candidates = analyze_warrant_scenario(
            &[weak, good],
            1533.5,
            &scenario(),
            &DecisionConfig::default(),
            NaiveDate::from_ymd_opt(2026, 5, 18).unwrap(),
        );

        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].symbol, "GOOD");
        assert_eq!(candidates[0].decision, "BUY");
    }

    #[test]
    fn helper_dates_and_distances_are_directional() {
        assert_eq!(
            default_max_maturity_for_early_next_year(
                NaiveDate::from_ymd_opt(2026, 10, 31).unwrap()
            ),
            Some(NaiveDate::from_ymd_opt(2027, 3, 31).unwrap())
        );
        assert_close(
            directional_distance_pct(OptionKind::Call, 100.0, 120.0),
            20.0,
            1e-9,
        );
        assert_close(
            directional_distance_pct(OptionKind::Put, 100.0, 80.0),
            20.0,
            1e-9,
        );
    }

    fn scenario() -> ScenarioConfig {
        ScenarioConfig {
            target_price: 1800.0,
            target_date: NaiveDate::from_ymd_opt(2026, 10, 31).unwrap(),
            min_maturity: Some(NaiveDate::from_ymd_opt(2027, 1, 1).unwrap()),
            max_maturity: Some(NaiveDate::from_ymd_opt(2027, 3, 31).unwrap()),
            side: "call".to_string(),
            risk_free_rate: 0.045,
            dividend_yield: 0.005,
            fallback_volatility: 0.35,
            volatility_shock_points: -5.0,
            real_world_drift: 0.08,
            fx_target_rate: None,
            fx_stress_pct: 0.0,
            vol_spot_slope_points_per_pct: -0.50,
            exit_spread_multiplier: 1.5,
            exit_spread_delta_penalty: 0.75,
            stale_pricing_guard: false,
            paris_hour: None,
            dividends: Vec::new(),
        }
    }

    fn signal() -> OpportunitySignal {
        OpportunitySignal {
            symbol: "RMSCALL".to_string(),
            web_url: "https://example.test".to_string(),
            side: "call".to_string(),
            moneyness: "otm".to_string(),
            product_family: "warrant".to_string(),
            pricing_model: "warrant_intrinsic".to_string(),
            product_type: "Warrant Call".to_string(),
            maturity: "2027-03-19".to_string(),
            price_currency: "EUR".to_string(),
            last_price: 1.20,
            price_source: "euronext_ask".to_string(),
            bid_price: Some(1.19),
            ask_price: Some(1.20),
            mid_price: Some(1.195),
            spread_pct: Some(0.84),
            bid_size: Some(1000.0),
            ask_size: Some(1000.0),
            quote_volume: Some(10.0),
            execution_status: "executable_bid_ask".to_string(),
            data_quality_score: 95.0,
            liquidity_score: 70.0,
            strike: 1700.0,
            strike_currency: "EUR".to_string(),
            barrier: None,
            barrier_currency: None,
            barrier_distance_pct: None,
            raw_intrinsic: 0.0,
            intrinsic_per_product: 0.0,
            intrinsic_currency: "EUR".to_string(),
            fx_rate: 1.0,
            fx_source_ticker: "IDENTITY".to_string(),
            warrants_per_underlying: Some(100.0),
            price_to_intrinsic: 0.0,
            effective_gearing: Some(10.0),
            premium_discount_pct: 0.0,
            years_to_maturity: Some(0.84),
            risk_free_rate: Some(0.045),
            dividend_yield: Some(0.005),
            implied_volatility: Some(0.35),
            smile_median_iv: Some(0.37),
            smile_gap_vol_points: Some(-2.0),
            volatility_signal: "iv_cheap".to_string(),
            metric_kind: "premium_pct".to_string(),
            relative_metric: 0.5,
            peer_median_relative_metric: 0.7,
            peer_gap_pct: 28.0,
            spread_adjusted_gap_pct: 27.0,
            peer_count: 8,
            valuation: ValuationSide::Undervalued,
            score: 27.0,
            note: String::new(),
        }
    }

    fn assert_close(actual: f64, expected: f64, tolerance: f64) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "actual={actual}, expected={expected}, tolerance={tolerance}"
        );
    }
}
