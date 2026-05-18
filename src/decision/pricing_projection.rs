use crate::decision::models::MarketContext;
use crate::indicators::options::{black_scholes_price, BlackScholesInput, OptionKind};
use crate::indicators::warrant::OpportunitySignal;

pub fn estimate_product_price_at_underlying(
    signal: &OpportunitySignal,
    underlying_price: f64,
    market: &MarketContext,
) -> Option<f64> {
    if !underlying_price.is_finite() || underlying_price <= 0.0 {
        return None;
    }

    let parity = signal.warrants_per_underlying.filter(|value| *value > 0.0)?;
    let fx = market
        .fx_rates
        .rate(&signal.strike_currency, &signal.price_currency)?;

    match signal.pricing_model.as_str() {
        "financing_level" | "barrier_only" => {
            let intrinsic = raw_intrinsic(signal.side.as_str(), underlying_price, signal.strike)?;
            Some(intrinsic / parity * fx)
        }
        "warrant_intrinsic" => estimate_warrant_price(signal, underlying_price, market, parity, fx),
        _ => None,
    }
}

fn estimate_warrant_price(
    signal: &OpportunitySignal,
    underlying_price: f64,
    market: &MarketContext,
    parity: f64,
    fx: f64,
) -> Option<f64> {
    if let (Some(iv), Some(years)) = (signal.implied_volatility, signal.years_to_maturity) {
        let kind = option_kind(signal.side.as_str())?;
        let price_per_underlying = black_scholes_price(BlackScholesInput {
            kind,
            spot: underlying_price,
            strike: signal.strike,
            years_to_maturity: years,
            risk_free_rate: signal.risk_free_rate.unwrap_or(market.risk_free_rate),
            dividend_yield: signal.dividend_yield.unwrap_or(market.dividend_yield),
            volatility: iv,
        })?;
        return Some(price_per_underlying / parity * fx);
    }

    let intrinsic = raw_intrinsic(signal.side.as_str(), underlying_price, signal.strike)?;
    let projected_intrinsic = intrinsic / parity * fx;
    let current_time_value = (signal.last_price - signal.intrinsic_per_product).max(0.0);
    Some(projected_intrinsic + current_time_value)
}

fn raw_intrinsic(side: &str, underlying_price: f64, reference: f64) -> Option<f64> {
    match side {
        "call" => Some((underlying_price - reference).max(0.0)),
        "put" => Some((reference - underlying_price).max(0.0)),
        _ => None,
    }
}

fn option_kind(side: &str) -> Option<OptionKind> {
    match side {
        "call" => Some(OptionKind::Call),
        "put" => Some(OptionKind::Put),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::models::MarketContext;
    use crate::indicators::warrant::{OpportunitySignal, ValuationSide};
    use crate::models::fx::{FxRate, FxRateBook};

    #[test]
    fn projects_eur_turbo_put_with_parity_200() {
        let signal = signal("put", "financing_level", 1800.0, "EUR", "EUR", 200.0);
        let market = market("EUR", FxRateBook::default());

        let price = estimate_product_price_at_underlying(&signal, 1575.5, &market).unwrap();

        assert_close(price, 1.1225, 1e-6);
    }

    #[test]
    fn projects_usd_turbo_put_quoted_in_eur_with_fx() {
        let mut fx = FxRateBook::default();
        fx.add(FxRate {
            from: "USD".to_string(),
            to: "EUR".to_string(),
            rate: 0.8602,
            source_ticker: "EURUSD=X".to_string(),
        });
        let signal = signal("put", "financing_level", 369.61, "USD", "EUR", 10.0);
        let market = market("USD", fx);

        let price = estimate_product_price_at_underlying(&signal, 300.23, &market).unwrap();

        assert_close(price, 5.9681, 1e-4);
    }

    #[test]
    fn projects_warrant_call_without_mixing_eur_and_usd() {
        let mut fx = FxRateBook::default();
        fx.add(FxRate {
            from: "USD".to_string(),
            to: "EUR".to_string(),
            rate: 0.8603,
            source_ticker: "EURUSD=X".to_string(),
        });
        let mut signal = signal("call", "warrant_intrinsic", 260.0, "USD", "EUR", 10.0);
        signal.last_price = 3.65;
        signal.intrinsic_per_product = (300.23 - 260.0) / 10.0 * 0.8603;
        let market = market("USD", fx);

        let price = estimate_product_price_at_underlying(&signal, 300.23, &market).unwrap();

        assert_close(price, 3.65, 1e-6);
    }

    fn market(currency: &str, fx_rates: FxRateBook) -> MarketContext {
        MarketContext {
            underlying_ticker: "TEST".to_string(),
            spot: 0.0,
            spot_currency: currency.to_string(),
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

    fn signal(
        side: &str,
        pricing_model: &str,
        strike: f64,
        strike_currency: &str,
        price_currency: &str,
        parity: f64,
    ) -> OpportunitySignal {
        OpportunitySignal {
            symbol: "TEST".to_string(),
            web_url: "https://example.test".to_string(),
            side: side.to_string(),
            moneyness: "itm".to_string(),
            product_family: "turbo".to_string(),
            pricing_model: pricing_model.to_string(),
            product_type: "Turbo".to_string(),
            maturity: "open-end".to_string(),
            price_currency: price_currency.to_string(),
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
            strike,
            strike_currency: strike_currency.to_string(),
            barrier: Some(strike),
            barrier_currency: Some(strike_currency.to_string()),
            barrier_distance_pct: Some(10.0),
            raw_intrinsic: 1.0,
            intrinsic_per_product: 1.0,
            intrinsic_currency: price_currency.to_string(),
            fx_rate: 1.0,
            fx_source_ticker: "IDENTITY".to_string(),
            warrants_per_underlying: Some(parity),
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
