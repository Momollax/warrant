use crate::decision::config::DecisionConfig;
use crate::decision::models::{MarketContext, StopPlan, TargetPlan, TargetReason};
use crate::decision::pricing_projection::estimate_product_price_at_underlying;
use crate::indicators::warrant::OpportunitySignal;

pub fn compute_targets(
    signal: &OpportunitySignal,
    stop: &StopPlan,
    market: &MarketContext,
    config: &DecisionConfig,
) -> Option<(TargetPlan, TargetPlan)> {
    let risk_underlying = (market.spot - stop.underlying_stop_price).abs();
    if risk_underlying <= 0.0 {
        return None;
    }

    let target_1_underlying = directional_target(
        signal,
        market.spot,
        risk_underlying * config.target_1_r_multiple,
    )?;
    let target_2_underlying = directional_target(
        signal,
        market.spot,
        risk_underlying * config.target_2_r_multiple,
    )?;

    let target_1 = build_target(signal, market, target_1_underlying)?;
    let target_2 = build_target(signal, market, target_2_underlying)?;
    Some((target_1, target_2))
}

pub fn compute_reward_risk(gain_pct: f64, loss_pct: f64) -> Option<f64> {
    (gain_pct.is_finite() && loss_pct.is_finite() && loss_pct > 0.0)
        .then_some(gain_pct / loss_pct.abs())
}

fn directional_target(signal: &OpportunitySignal, spot: f64, distance: f64) -> Option<f64> {
    match signal.side.as_str() {
        "call" => Some(spot + distance),
        "put" => Some(spot - distance),
        _ => None,
    }
}

fn build_target(
    signal: &OpportunitySignal,
    market: &MarketContext,
    underlying_target_price: f64,
) -> Option<TargetPlan> {
    let estimated_product_target_price =
        estimate_product_price_at_underlying(signal, underlying_target_price, market)?;
    let gain_pct = (estimated_product_target_price - signal.last_price) / signal.last_price * 100.0;
    let distance_to_target_pct =
        (underlying_target_price - market.spot).abs() / market.spot * 100.0;

    Some(TargetPlan {
        underlying_target_price,
        estimated_product_target_price,
        gain_pct,
        distance_to_target_pct,
        target_reason: TargetReason::RiskMultiple,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::models::{MarketContext, StopPlan, StopReason};
    use crate::indicators::warrant::{OpportunitySignal, ValuationSide};
    use crate::models::fx::FxRateBook;

    #[test]
    fn put_targets_move_below_spot_and_compute_reward_risk() {
        let signal = signal();
        let market = market();
        let stop = StopPlan {
            underlying_stop_price: 1620.5,
            estimated_product_stop_price: 0.8975,
            loss_pct: 24.0,
            distance_to_stop_pct: 2.86,
            stop_reason: StopReason::Atr,
        };

        let (target_1, target_2) =
            compute_targets(&signal, &stop, &market, &DecisionConfig::default()).unwrap();
        let rr = compute_reward_risk(target_2.gain_pct, stop.loss_pct).unwrap();

        assert!(target_1.underlying_target_price < market.spot);
        assert!(target_2.underlying_target_price < target_1.underlying_target_price);
        assert!(target_1.gain_pct > 0.0);
        assert!(target_2.gain_pct > target_1.gain_pct);
        assert!(rr > 1.0);
    }

    fn market() -> MarketContext {
        MarketContext {
            underlying_ticker: "RMS.PA".to_string(),
            spot: 1575.5,
            spot_currency: "EUR".to_string(),
            change_pct: 0.0,
            fx_rates: FxRateBook::default(),
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

    fn signal() -> OpportunitySignal {
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
            spread_pct: Some(8.85),
            bid_size: Some(100.0),
            ask_size: Some(100.0),
            quote_volume: Some(0.0),
            execution_status: "executable_bid_ask".to_string(),
            data_quality_score: 60.0,
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
            spread_adjusted_gap_pct: -4.15,
            peer_count: 8,
            valuation: ValuationSide::Undervalued,
            score: 0.0,
            note: String::new(),
        }
    }
}
