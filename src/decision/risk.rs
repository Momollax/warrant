use crate::decision::config::DecisionConfig;
use crate::decision::models::{MarketContext, StopReason};
use crate::indicators::warrant::OpportunitySignal;

#[derive(Debug, Clone, Copy)]
pub struct StopCandidate {
    pub price: f64,
    pub reason: StopReason,
}

pub fn compute_breakeven_move_pct(
    spread_pct: Option<f64>,
    effective_gearing: Option<f64>,
) -> Option<f64> {
    let spread_pct = spread_pct?;
    let effective_gearing = effective_gearing?;
    (spread_pct.is_finite()
        && spread_pct >= 0.0
        && effective_gearing.is_finite()
        && effective_gearing > 0.0)
        .then_some(spread_pct / effective_gearing)
}

pub fn compute_barrier_risk_score(
    barrier_distance_pct: Option<f64>,
    effective_gearing: Option<f64>,
) -> Option<f64> {
    let barrier_distance_pct = barrier_distance_pct?;
    let effective_gearing = effective_gearing?;
    (barrier_distance_pct > 0.0 && effective_gearing > 0.0)
        .then_some(effective_gearing / barrier_distance_pct)
}

pub fn compute_underlying_stop(
    signal: &OpportunitySignal,
    market: &MarketContext,
    config: &DecisionConfig,
) -> Option<StopCandidate> {
    let mut candidate = if is_call(signal) {
        if let Some(support) = market.support_1.filter(|value| *value > 0.0 && *value < market.spot)
        {
            StopCandidate {
                price: support,
                reason: StopReason::TechnicalSupportResistance,
            }
        } else if let Some(atr) = market.atr_14d.filter(|value| *value > 0.0) {
            StopCandidate {
                price: market.spot - atr * config.stop_atr_multiple,
                reason: StopReason::Atr,
            }
        } else {
            StopCandidate {
                price: market.spot * 0.97,
                reason: StopReason::FallbackPercent,
            }
        }
    } else if is_put(signal) {
        if let Some(resistance) = market
            .resistance_1
            .filter(|value| *value > market.spot)
        {
            StopCandidate {
                price: resistance,
                reason: StopReason::TechnicalSupportResistance,
            }
        } else if let Some(atr) = market.atr_14d.filter(|value| *value > 0.0) {
            StopCandidate {
                price: market.spot + atr * config.stop_atr_multiple,
                reason: StopReason::Atr,
            }
        } else {
            StopCandidate {
                price: market.spot * 1.03,
                reason: StopReason::FallbackPercent,
            }
        }
    } else {
        return None;
    };

    candidate = apply_barrier_buffer(signal, market.spot, candidate, config)?;
    valid_stop_direction(signal, market.spot, candidate.price).then_some(candidate)
}

fn apply_barrier_buffer(
    signal: &OpportunitySignal,
    spot: f64,
    candidate: StopCandidate,
    config: &DecisionConfig,
) -> Option<StopCandidate> {
    let Some(barrier) = signal.barrier else {
        return Some(candidate);
    };
    if barrier <= 0.0 {
        return Some(candidate);
    }

    let buffer = config.barrier_stop_buffer_pct / 100.0;
    if is_call(signal) {
        let minimum_stop = barrier * (1.0 + buffer);
        if candidate.price <= minimum_stop {
            let adjusted = StopCandidate {
                price: minimum_stop,
                reason: StopReason::BarrierBuffer,
            };
            return (adjusted.price < spot).then_some(adjusted);
        }
    } else if is_put(signal) {
        let maximum_stop = barrier * (1.0 - buffer);
        if candidate.price >= maximum_stop {
            let adjusted = StopCandidate {
                price: maximum_stop,
                reason: StopReason::BarrierBuffer,
            };
            return (adjusted.price > spot).then_some(adjusted);
        }
    }

    Some(candidate)
}

pub fn distance_to_stop_pct(signal: &OpportunitySignal, spot: f64, stop: f64) -> Option<f64> {
    if spot <= 0.0 || stop <= 0.0 {
        return None;
    }
    if is_call(signal) {
        Some((spot - stop) / spot * 100.0)
    } else if is_put(signal) {
        Some((stop - spot) / spot * 100.0)
    } else {
        None
    }
}

pub fn product_loss_pct(entry_price: f64, stop_price: f64) -> Option<f64> {
    (entry_price > 0.0 && stop_price >= 0.0)
        .then_some((entry_price - stop_price) / entry_price * 100.0)
}

pub fn entry_breakeven_underlying_price(
    signal: &OpportunitySignal,
    entry_price: f64,
    market: &MarketContext,
) -> Option<f64> {
    if !entry_price.is_finite() || entry_price <= 0.0 {
        return None;
    }
    if !matches!(signal.pricing_model.as_str(), "financing_level" | "barrier_only") {
        return None;
    }

    let parity = signal.warrants_per_underlying.filter(|value| *value > 0.0)?;
    let fx = market
        .fx_rates
        .rate(&signal.strike_currency, &signal.price_currency)?;
    if fx <= 0.0 {
        return None;
    }

    let reference_move = entry_price / fx * parity;
    if is_call(signal) {
        Some(signal.strike + reference_move)
    } else if is_put(signal) {
        Some(signal.strike - reference_move)
    } else {
        None
    }
}

pub fn adjust_stop_beyond_entry_breakeven(
    signal: &OpportunitySignal,
    market: &MarketContext,
    entry_price: f64,
    config: &DecisionConfig,
) -> Option<StopCandidate> {
    let breakeven = entry_breakeven_underlying_price(signal, entry_price, market)?;
    let buffer = (market.spot.abs() * 0.001).max(0.01);
    let candidate = if is_call(signal) {
        StopCandidate {
            price: breakeven - buffer,
            reason: StopReason::EntryBreakeven,
        }
    } else if is_put(signal) {
        StopCandidate {
            price: breakeven + buffer,
            reason: StopReason::EntryBreakeven,
        }
    } else {
        return None;
    };

    let candidate = apply_barrier_buffer(signal, market.spot, candidate, config)?;
    valid_stop_direction(signal, market.spot, candidate.price).then_some(candidate)
}

fn valid_stop_direction(signal: &OpportunitySignal, spot: f64, stop: f64) -> bool {
    (is_call(signal) && stop < spot) || (is_put(signal) && stop > spot)
}

fn is_call(signal: &OpportunitySignal) -> bool {
    signal.side == "call"
}

fn is_put(signal: &OpportunitySignal) -> bool {
    signal.side == "put"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::models::MarketContext;
    use crate::indicators::warrant::{OpportunitySignal, ValuationSide};
    use crate::models::fx::FxRateBook;

    #[test]
    fn breakeven_move_is_spread_divided_by_gearing() {
        let value = compute_breakeven_move_pct(Some(8.85), Some(6.67)).unwrap();
        assert_close(value, 1.327, 1e-3);
    }

    #[test]
    fn put_stop_uses_resistance_when_available() {
        let signal = signal("put", Some(1800.0));
        let market = market(1575.5, None, Some(1620.0), None);

        let stop = compute_underlying_stop(&signal, &market, &DecisionConfig::default()).unwrap();

        assert_eq!(stop.reason, StopReason::TechnicalSupportResistance);
        assert_eq!(stop.price, 1620.0);
    }

    #[test]
    fn call_stop_uses_atr_when_support_is_missing() {
        let signal = signal("call", Some(1400.0));
        let market = market(1575.5, None, None, Some(20.0));

        let stop = compute_underlying_stop(&signal, &market, &DecisionConfig::default()).unwrap();

        assert_eq!(stop.reason, StopReason::Atr);
        assert_close(stop.price, 1545.5, 1e-9);
    }

    #[test]
    fn put_stop_is_kept_before_barrier_buffer() {
        let signal = signal("put", Some(1800.0));
        let market = market(1575.5, None, Some(1795.0), None);

        let stop = compute_underlying_stop(&signal, &market, &DecisionConfig::default()).unwrap();

        assert_eq!(stop.reason, StopReason::BarrierBuffer);
        assert_close(stop.price, 1764.0, 1e-9);
    }

    #[test]
    fn put_entry_breakeven_uses_fx_and_parity() {
        let mut fx = FxRateBook::default();
        fx.add(crate::models::fx::FxRate {
            from: "USD".to_string(),
            to: "EUR".to_string(),
            rate: 0.8606,
            source_ticker: "EURUSD=X".to_string(),
        });
        let mut signal = signal("put", Some(351.10));
        signal.strike = 369.61;
        signal.strike_currency = "USD".to_string();
        signal.price_currency = "EUR".to_string();
        signal.warrants_per_underlying = Some(10.0);
        let market = MarketContext {
            spot: 300.23,
            spot_currency: "USD".to_string(),
            fx_rates: fx,
            ..market(300.23, None, Some(300.92), None)
        };

        let breakeven = entry_breakeven_underlying_price(&signal, 5.8320, &market).unwrap();

        assert_close(breakeven, 301.8433, 1e-3);
    }

    #[test]
    fn adjusts_put_stop_beyond_entry_breakeven() {
        let mut fx = FxRateBook::default();
        fx.add(crate::models::fx::FxRate {
            from: "USD".to_string(),
            to: "EUR".to_string(),
            rate: 0.8606,
            source_ticker: "EURUSD=X".to_string(),
        });
        let mut signal = signal("put", Some(351.10));
        signal.strike = 369.61;
        signal.strike_currency = "USD".to_string();
        signal.price_currency = "EUR".to_string();
        signal.warrants_per_underlying = Some(10.0);
        let market = MarketContext {
            spot: 300.23,
            spot_currency: "USD".to_string(),
            fx_rates: fx,
            ..market(300.23, None, Some(300.92), None)
        };

        let stop =
            adjust_stop_beyond_entry_breakeven(&signal, &market, 5.8320, &DecisionConfig::default())
                .unwrap();

        assert_eq!(stop.reason, StopReason::EntryBreakeven);
        assert!(stop.price > 301.84);
        assert!(stop.price < 351.10);
    }

    fn market(
        spot: f64,
        support_1: Option<f64>,
        resistance_1: Option<f64>,
        atr_14d: Option<f64>,
    ) -> MarketContext {
        MarketContext {
            underlying_ticker: "TEST".to_string(),
            spot,
            spot_currency: "EUR".to_string(),
            change_pct: 0.0,
            fx_rates: FxRateBook::default(),
            realized_volatility_20d: None,
            atr_14d,
            support_1,
            support_2: None,
            resistance_1,
            resistance_2: None,
            risk_free_rate: 0.045,
            dividend_yield: 0.005,
        }
    }

    fn signal(side: &str, barrier: Option<f64>) -> OpportunitySignal {
        OpportunitySignal {
            symbol: "TEST".to_string(),
            web_url: "https://example.test".to_string(),
            side: side.to_string(),
            moneyness: "itm".to_string(),
            product_family: "turbo".to_string(),
            pricing_model: "financing_level".to_string(),
            product_type: "Turbo".to_string(),
            maturity: "open-end".to_string(),
            price_currency: "EUR".to_string(),
            last_price: 1.0,
            price_source: "boursorama_ask".to_string(),
            bid_price: Some(0.99),
            ask_price: Some(1.0),
            mid_price: Some(0.995),
            spread_pct: Some(1.0),
            bid_size: Some(100.0),
            ask_size: Some(100.0),
            quote_volume: Some(10.0),
            execution_status: "executable_bid_ask".to_string(),
            data_quality_score: 95.0,
            liquidity_score: 70.0,
            strike: barrier.unwrap_or(1800.0),
            strike_currency: "EUR".to_string(),
            barrier,
            barrier_currency: Some("EUR".to_string()),
            barrier_distance_pct: Some(10.0),
            raw_intrinsic: 1.0,
            intrinsic_per_product: 1.0,
            intrinsic_currency: "EUR".to_string(),
            fx_rate: 1.0,
            fx_source_ticker: "IDENTITY".to_string(),
            warrants_per_underlying: Some(100.0),
            price_to_intrinsic: 1.0,
            effective_gearing: Some(5.0),
            premium_discount_pct: 0.0,
            years_to_maturity: None,
            risk_free_rate: None,
            dividend_yield: None,
            implied_volatility: None,
            smile_median_iv: None,
            smile_gap_vol_points: None,
            volatility_signal: "no_iv".to_string(),
            metric_kind: "price_to_intrinsic".to_string(),
            relative_metric: 1.0,
            peer_median_relative_metric: 1.0,
            peer_gap_pct: 2.0,
            spread_adjusted_gap_pct: 1.0,
            peer_count: 5,
            valuation: ValuationSide::Undervalued,
            score: 1.0,
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
