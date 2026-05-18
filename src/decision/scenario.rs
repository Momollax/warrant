use chrono::{Datelike, NaiveDate};

use crate::decision::config::DecisionConfig;
use crate::indicators::options::{
    black_scholes_greeks, black_scholes_price, lognormal_probability, BlackScholesInput,
    OptionKind,
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
    pub net_return_pct: f64,
    pub gross_return_pct: f64,
    pub fee_drag_pct: f64,
    pub breakeven_underlying: Option<f64>,
    pub breakeven_distance_pct: Option<f64>,
    pub implied_volatility: Option<f64>,
    pub volatility_used: f64,
    pub theta_horizon_pct: Option<f64>,
    pub spread_pct: Option<f64>,
    pub probability_target_pct: Option<f64>,
    pub probability_breakeven_pct: Option<f64>,
    pub target_zscore: Option<f64>,
    pub breakeven_zscore: Option<f64>,
    pub expected_value_pct: Option<f64>,
    pub sharpe_like: Option<f64>,
    pub kelly_fraction_pct: Option<f64>,
    pub delta: Option<f64>,
    pub gamma: Option<f64>,
    pub vega_per_vol_point: Option<f64>,
    pub theta_per_day: Option<f64>,
    pub rho_per_rate_point: Option<f64>,
    pub d1: Option<f64>,
    pub d2: Option<f64>,
    pub data_quality_score: f64,
    pub liquidity_score: f64,
    pub parity: f64,
    pub reasons: Vec<String>,
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
    let years_remaining = years_between(config.target_date, maturity)?;
    let years_to_target = years_between(today, config.target_date)?;
    let years_to_maturity = years_between(today, maturity)?;
    let volatility = signal
        .implied_volatility
        .or(signal.smile_median_iv)
        .unwrap_or(config.fallback_volatility)
        .clamp(0.0001, 5.0);
    let projected_price = scenario_product_price(
        kind,
        config.target_price,
        signal.strike,
        years_remaining,
        config.risk_free_rate,
        config.dividend_yield,
        volatility,
        parity,
        signal.fx_rate,
    )?;
    if projected_price <= 0.0 {
        return None;
    }

    let gross_return_pct = (projected_price - entry_price) / entry_price * 100.0;
    let fee_result = net_return_after_fees(
        entry_price,
        projected_price,
        decision_config.fee_order_notional,
        decision_config.fee_buy_fixed,
        decision_config.fee_buy_pct,
        decision_config.fee_sell_fixed,
        decision_config.fee_sell_pct,
        decision_config.fee_deposit_fixed,
        decision_config.fee_deposit_pct,
    )?;
    let breakeven_underlying =
        solve_breakeven_underlying(kind, signal, years_remaining, volatility, config, decision_config);
    let breakeven_distance_pct =
        breakeven_underlying.map(|value| directional_distance_pct(kind, spot, value));
    let theta_horizon_pct = theta_horizon_pct(signal, entry_price, projected_price, today, config.target_date);
    let probability = scenario_probabilities(
        kind,
        spot,
        config.target_price,
        breakeven_underlying,
        years_to_target,
        config.risk_free_rate - config.dividend_yield,
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
        -100.0,
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

    let mut reasons = Vec::new();
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

    let score = scenario_score(
        fee_result.net_return_pct,
        signal.data_quality_score,
        signal.spread_pct,
        signal.smile_gap_vol_points,
        maturity_fit_score(config.target_date, maturity),
    );
    let decision = if reasons.is_empty()
        && fee_result.net_return_pct >= 20.0
        && score >= 70.0
    {
        "BUY"
    } else if fee_result.net_return_pct > 0.0 && reasons.len() <= 1 {
        "WATCH"
    } else {
        "AVOID"
    };

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
        net_return_pct: fee_result.net_return_pct,
        gross_return_pct,
        fee_drag_pct: fee_result.fee_drag_pct,
        breakeven_underlying,
        breakeven_distance_pct,
        implied_volatility: signal.implied_volatility,
        volatility_used: volatility,
        theta_horizon_pct,
        spread_pct: signal.spread_pct,
        probability_target_pct: probability.probability_target_pct,
        probability_breakeven_pct: probability.probability_breakeven_pct,
        target_zscore: probability.target_zscore,
        breakeven_zscore: probability.breakeven_zscore,
        expected_value_pct: risk_neutral_ev_pct,
        sharpe_like: binary_stats.sharpe_like,
        kelly_fraction_pct: binary_stats.kelly_fraction_pct,
        delta: greeks.as_ref().map(|value| value.delta),
        gamma: greeks.as_ref().map(|value| value.gamma),
        vega_per_vol_point: greeks.as_ref().map(|value| value.vega_per_vol_point),
        theta_per_day: greeks.as_ref().map(|value| value.theta_per_day),
        rho_per_rate_point: greeks.as_ref().map(|value| value.rho_per_rate_point),
        d1: greeks.as_ref().map(|value| value.d1),
        d2: greeks.as_ref().map(|value| value.d2),
        data_quality_score: signal.data_quality_score,
        liquidity_score: signal.liquidity_score,
        parity,
        reasons,
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

#[derive(Debug, Clone, Copy, Default)]
struct ScenarioProbabilities {
    probability_target_pct: Option<f64>,
    probability_breakeven_pct: Option<f64>,
    target_zscore: Option<f64>,
    breakeven_zscore: Option<f64>,
}

fn scenario_probabilities(
    kind: OptionKind,
    spot: f64,
    target_price: f64,
    breakeven_underlying: Option<f64>,
    years_to_target: f64,
    drift: f64,
    volatility: f64,
) -> ScenarioProbabilities {
    let target = lognormal_probability(kind, spot, target_price, years_to_target, drift, volatility)
        .map(|(probability, zscore)| (probability * 100.0, zscore));
    let breakeven = breakeven_underlying.and_then(|level| {
        lognormal_probability(kind, spot, level, years_to_target, drift, volatility)
            .map(|(probability, zscore)| (probability * 100.0, zscore))
    });

    ScenarioProbabilities {
        probability_target_pct: target.map(|value| value.0),
        target_zscore: target.map(|value| value.1),
        probability_breakeven_pct: breakeven.map(|value| value.0),
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
        let price = scenario_product_price(
            kind,
            mid,
            signal.strike,
            years_remaining,
            config.risk_free_rate,
            config.dividend_yield,
            volatility,
            signal.warrants_per_underlying?,
            signal.fx_rate,
        )?;
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

fn theta_horizon_pct(
    signal: &OpportunitySignal,
    entry_price: f64,
    projected_price: f64,
    today: NaiveDate,
    target_date: NaiveDate,
) -> Option<f64> {
    let days = (target_date - today).num_days().max(1) as f64;
    let intrinsic_now = signal.intrinsic_per_product.max(0.0);
    let time_value_now = (entry_price - intrinsic_now).max(0.0);
    let projected_intrinsic = match signal.side.as_str() {
        "call" => (projected_price - intrinsic_now).max(0.0),
        "put" => (projected_price - intrinsic_now).max(0.0),
        _ => 0.0,
    };
    let decay = (time_value_now - projected_intrinsic).max(0.0);
    Some(decay / entry_price * 100.0 / days)
}

fn scenario_score(
    net_return_pct: f64,
    data_quality_score: f64,
    spread_pct: Option<f64>,
    smile_gap_vol_points: Option<f64>,
    maturity_fit_score: f64,
) -> f64 {
    let return_score = (net_return_pct.max(0.0) / 100.0 * 100.0).clamp(0.0, 100.0);
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
        assert!(candidate.d1.unwrap() > candidate.d2.unwrap());
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
