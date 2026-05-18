use crate::decision::config::DecisionConfig;
use crate::decision::models::{HorizonPlan, MarketContext};
use crate::indicators::options::{black_scholes_theta_per_year, BlackScholesInput, OptionKind};
use crate::indicators::warrant::OpportunitySignal;

pub fn compute_horizon_plan(
    signal: &OpportunitySignal,
    market: &MarketContext,
    entry_price: f64,
    config: &DecisionConfig,
) -> HorizonPlan {
    let maturity_days = signal.years_to_maturity.map(|years| (years * 365.0).round() as i64);
    let theta_daily_pct = estimate_theta_daily_pct(signal, market, entry_price);
    let theta_to_horizon_pct = theta_daily_pct.map(|theta| theta * config.default_holding_days as f64);
    let time_risk_score = compute_time_risk_score(
        signal.maturity.as_str(),
        maturity_days,
        theta_to_horizon_pct,
        config,
    );

    HorizonPlan {
        holding_days: config.default_holding_days,
        max_holding_days: config.max_holding_days,
        maturity_days,
        theta_daily_pct,
        theta_to_horizon_pct,
        time_risk_score,
    }
}

fn estimate_theta_daily_pct(
    signal: &OpportunitySignal,
    market: &MarketContext,
    entry_price: f64,
) -> Option<f64> {
    if signal.pricing_model != "warrant_intrinsic" || entry_price <= 0.0 {
        return None;
    }

    let years = signal.years_to_maturity?;
    let volatility = signal.implied_volatility?;
    let parity = signal.warrants_per_underlying.filter(|value| *value > 0.0)?;
    let fx = market
        .fx_rates
        .rate(&signal.strike_currency, &signal.price_currency)?;
    let kind = match signal.side.as_str() {
        "call" => OptionKind::Call,
        "put" => OptionKind::Put,
        _ => return None,
    };

    let theta_per_underlying_year = black_scholes_theta_per_year(BlackScholesInput {
        kind,
        spot: market.spot,
        strike: signal.strike,
        years_to_maturity: years,
        risk_free_rate: signal.risk_free_rate.unwrap_or(market.risk_free_rate),
        dividend_yield: signal.dividend_yield.unwrap_or(market.dividend_yield),
        volatility,
    })?;
    let theta_per_product_day = theta_per_underlying_year / 365.0 / parity * fx;
    let decay_cost = (-theta_per_product_day).max(0.0);
    Some(decay_cost / entry_price * 100.0)
}

fn compute_time_risk_score(
    maturity: &str,
    maturity_days: Option<i64>,
    theta_to_horizon_pct: Option<f64>,
    config: &DecisionConfig,
) -> f64 {
    let maturity_score = match (maturity, maturity_days) {
        ("open-end", _) => 90.0,
        (_, Some(days)) if days <= 0 => 0.0,
        (_, Some(days)) => (days as f64 / config.min_maturity_days as f64 * 100.0).clamp(0.0, 100.0),
        _ => 65.0,
    };

    let theta_score = theta_to_horizon_pct
        .map(|theta| 100.0 - (theta / config.max_theta_to_horizon_pct * 100.0).clamp(0.0, 100.0))
        .unwrap_or(80.0);

    maturity_score * 0.65 + theta_score * 0.35
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indicators::warrant::ValuationSide;
    use crate::models::fx::{FxRate, FxRateBook};

    #[test]
    fn open_end_turbo_has_no_theta_and_high_time_score() {
        let signal = turbo_signal();
        let market = market(FxRateBook::default());

        let horizon = compute_horizon_plan(&signal, &market, 1.18, &DecisionConfig::default());

        assert_eq!(horizon.theta_daily_pct, None);
        assert!(horizon.time_risk_score > 80.0);
    }

    #[test]
    fn warrant_with_iv_has_theta_cost() {
        let mut fx = FxRateBook::default();
        fx.add(FxRate {
            from: "USD".to_string(),
            to: "EUR".to_string(),
            rate: 0.86,
            source_ticker: "EURUSD=X".to_string(),
        });
        let mut signal = turbo_signal();
        signal.side = "call".to_string();
        signal.pricing_model = "warrant_intrinsic".to_string();
        signal.strike = 260.0;
        signal.strike_currency = "USD".to_string();
        signal.price_currency = "EUR".to_string();
        signal.intrinsic_currency = "EUR".to_string();
        signal.years_to_maturity = Some(60.0 / 365.0);
        signal.implied_volatility = Some(0.35);
        signal.warrants_per_underlying = Some(10.0);
        let market = MarketContext {
            spot: 300.23,
            spot_currency: "USD".to_string(),
            fx_rates: fx,
            ..market(FxRateBook::default())
        };

        let horizon = compute_horizon_plan(&signal, &market, 3.65, &DecisionConfig::default());

        assert!(horizon.theta_daily_pct.unwrap() >= 0.0);
        assert!(horizon.theta_to_horizon_pct.unwrap() >= 0.0);
    }

    fn market(fx_rates: FxRateBook) -> MarketContext {
        MarketContext {
            underlying_ticker: "RMS.PA".to_string(),
            spot: 1575.5,
            spot_currency: "EUR".to_string(),
            change_pct: 0.0,
            fx_rates,
            realized_volatility_20d: None,
            atr_14d: None,
            support_1: None,
            support_2: None,
            resistance_1: None,
            resistance_2: None,
            risk_free_rate: 0.045,
            dividend_yield: 0.005,
        }
    }

    fn turbo_signal() -> OpportunitySignal {
        OpportunitySignal {
            symbol: "TEST".to_string(),
            web_url: "https://example.test".to_string(),
            side: "put".to_string(),
            moneyness: "itm".to_string(),
            product_family: "turbo".to_string(),
            pricing_model: "financing_level".to_string(),
            product_type: "Turbo Put".to_string(),
            maturity: "open-end".to_string(),
            price_currency: "EUR".to_string(),
            last_price: 1.18,
            price_source: "boursorama_ask".to_string(),
            bid_price: Some(1.08),
            ask_price: Some(1.18),
            mid_price: Some(1.13),
            spread_pct: Some(1.0),
            bid_size: Some(100.0),
            ask_size: Some(100.0),
            quote_volume: Some(10.0),
            execution_status: "executable_bid_ask".to_string(),
            data_quality_score: 95.0,
            liquidity_score: 70.0,
            strike: 1800.0,
            strike_currency: "EUR".to_string(),
            barrier: Some(1800.0),
            barrier_currency: Some("EUR".to_string()),
            barrier_distance_pct: Some(14.21),
            raw_intrinsic: 224.5,
            intrinsic_per_product: 1.1225,
            intrinsic_currency: "EUR".to_string(),
            fx_rate: 1.0,
            fx_source_ticker: "IDENTITY".to_string(),
            warrants_per_underlying: Some(200.0),
            price_to_intrinsic: 1.0512,
            effective_gearing: Some(6.76),
            premium_discount_pct: 5.12,
            years_to_maturity: None,
            risk_free_rate: None,
            dividend_yield: None,
            implied_volatility: None,
            smile_median_iv: None,
            smile_gap_vol_points: None,
            volatility_signal: "no_iv".to_string(),
            metric_kind: "price_to_intrinsic".to_string(),
            relative_metric: 1.0512,
            peer_median_relative_metric: 1.1030,
            peer_gap_pct: 4.70,
            spread_adjusted_gap_pct: 3.70,
            peer_count: 8,
            valuation: ValuationSide::Undervalued,
            score: 3.70,
            note: String::new(),
        }
    }
}
