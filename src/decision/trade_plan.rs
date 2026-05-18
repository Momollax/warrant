use crate::decision::config::DecisionConfig;
use crate::decision::models::{
    DecisionAction, DecisionSignal, MarketContext, RiskPlan, StopPlan, TradePlan,
};
use crate::decision::fees::compute_fee_plan;
use crate::decision::position::compute_position_plan;
use crate::decision::pricing_projection::estimate_product_price_at_underlying;
use crate::decision::risk::{
    adjust_stop_beyond_entry_breakeven, compute_barrier_risk_score, compute_breakeven_move_pct,
    compute_underlying_stop, distance_to_stop_pct, product_loss_pct,
};
use crate::decision::scoring::{compute_confidence_score, decide_action};
use crate::decision::targets::{compute_reward_risk, compute_targets};
use crate::decision::time::compute_horizon_plan;
use crate::indicators::warrant::OpportunitySignal;

pub fn build_trade_plan(
    signal: &OpportunitySignal,
    market: &MarketContext,
    config: &DecisionConfig,
) -> DecisionSignal {
    let Some(entry_price) = entry_price_for_long(signal) else {
        return DecisionSignal {
            opportunity: signal.clone(),
            trade_plan: None,
            decision: DecisionAction::Avoid,
            reasons: vec!["entry_price_not_executable".to_string()],
            warnings: Vec::new(),
        };
    };

    let Some(mut stop_candidate) = compute_underlying_stop(signal, market, config) else {
        return DecisionSignal {
            opportunity: signal.clone(),
            trade_plan: None,
            decision: DecisionAction::Avoid,
            reasons: vec!["stop_not_computable".to_string()],
            warnings: Vec::new(),
        };
    };

    let Some(mut stop_product_price) =
        estimate_product_price_at_underlying(signal, stop_candidate.price, market)
    else {
        return DecisionSignal {
            opportunity: signal.clone(),
            trade_plan: None,
            decision: DecisionAction::Avoid,
            reasons: vec!["stop_projection_not_computable".to_string()],
            warnings: Vec::new(),
        };
    };

    let mut stop_loss_pct = product_loss_pct(entry_price, stop_product_price).unwrap_or(100.0);
    if stop_loss_pct <= 0.0 {
        let Some(adjusted_stop_candidate) =
            adjust_stop_beyond_entry_breakeven(signal, market, entry_price, config)
        else {
            return DecisionSignal {
                opportunity: signal.clone(),
                trade_plan: None,
                decision: DecisionAction::Avoid,
                reasons: vec!["stop_projection_does_not_reduce_product_price".to_string()],
                warnings: Vec::new(),
            };
        };
        let Some(adjusted_stop_product_price) =
            estimate_product_price_at_underlying(signal, adjusted_stop_candidate.price, market)
        else {
            return DecisionSignal {
                opportunity: signal.clone(),
                trade_plan: None,
                decision: DecisionAction::Avoid,
                reasons: vec!["stop_projection_not_computable".to_string()],
                warnings: Vec::new(),
            };
        };
        let adjusted_stop_loss_pct =
            product_loss_pct(entry_price, adjusted_stop_product_price).unwrap_or(0.0);
        if adjusted_stop_loss_pct <= 0.0 {
            return DecisionSignal {
                opportunity: signal.clone(),
                trade_plan: None,
                decision: DecisionAction::Avoid,
                reasons: vec!["stop_projection_does_not_reduce_product_price".to_string()],
                warnings: Vec::new(),
            };
        }

        stop_candidate = adjusted_stop_candidate;
        stop_product_price = adjusted_stop_product_price;
        stop_loss_pct = adjusted_stop_loss_pct;
    }
    let mut stop = StopPlan {
        underlying_stop_price: stop_candidate.price,
        estimated_product_stop_price: stop_product_price,
        loss_pct: stop_loss_pct,
        distance_to_stop_pct: distance_to_stop_pct(signal, market.spot, stop_candidate.price)
            .unwrap_or(0.0),
        stop_reason: stop_candidate.reason,
    };

    let Some((mut target_1, mut target_2)) = compute_targets(signal, &stop, market, config) else {
        return DecisionSignal {
            opportunity: signal.clone(),
            trade_plan: None,
            decision: DecisionAction::Avoid,
            reasons: vec!["targets_not_computable".to_string()],
            warnings: Vec::new(),
        };
    };

    let Some(fees) = compute_fee_plan(entry_price, &stop, &target_1, &target_2, config) else {
        return DecisionSignal {
            opportunity: signal.clone(),
            trade_plan: None,
            decision: DecisionAction::Avoid,
            reasons: vec!["fees_not_computable".to_string()],
            warnings: Vec::new(),
        };
    };
    stop.loss_pct = fees.net_stop_loss_pct;
    target_1.gain_pct = fees.net_target_1_gain_pct;
    target_2.gain_pct = fees.net_target_2_gain_pct;

    let reward_risk_1 = compute_reward_risk(target_1.gain_pct, stop.loss_pct);
    let reward_risk_2 = compute_reward_risk(target_2.gain_pct, stop.loss_pct);
    let horizon = compute_horizon_plan(signal, market, entry_price, config);
    let confidence_score =
        compute_confidence_score(signal, reward_risk_2, horizon.time_risk_score, config);
    let risk = RiskPlan {
        barrier_distance_pct: signal.barrier_distance_pct,
        barrier_risk_score: compute_barrier_risk_score(
            signal.barrier_distance_pct,
            signal.effective_gearing,
        ),
        spread_pct: signal.spread_pct,
        spread_cost_underlying_pct: compute_breakeven_move_pct(
            signal.spread_pct,
            signal.effective_gearing,
        ),
        data_quality_score: signal.data_quality_score,
        liquidity_score: signal.liquidity_score,
        max_loss_pct: stop.loss_pct,
    };
    let Some(position) = compute_position_plan(entry_price, stop.loss_pct, config) else {
        return DecisionSignal {
            opportunity: signal.clone(),
            trade_plan: None,
            decision: DecisionAction::Avoid,
            reasons: vec!["position_sizing_not_computable".to_string()],
            warnings: Vec::new(),
        };
    };

    let trade_plan = TradePlan {
        entry_price,
        entry_price_source: signal.price_source.clone(),
        entry_underlying_price: market.spot,
        entry_edge_pct: signal.spread_adjusted_gap_pct,
        breakeven_move_pct: compute_breakeven_move_pct(
            signal.spread_pct,
            signal.effective_gearing,
        ),
        stop,
        target_1,
        target_2,
        reward_risk_1,
        reward_risk_2,
        horizon,
        risk,
        position,
        fees,
        confidence_score,
    };

    let (decision, reasons, warnings) = decide_action(signal, &trade_plan, config);
    DecisionSignal {
        opportunity: signal.clone(),
        trade_plan: Some(trade_plan),
        decision,
        reasons,
        warnings,
    }
}

pub fn entry_price_for_long(signal: &OpportunitySignal) -> Option<f64> {
    if signal.execution_status != "executable_bid_ask" {
        return None;
    }
    let bid = signal.bid_price?;
    let ask = signal.ask_price?;
    (bid > 0.0 && ask > 0.0 && ask >= bid).then_some(ask)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::models::MarketContext;
    use crate::indicators::warrant::{OpportunitySignal, ValuationSide};
    use crate::models::fx::FxRateBook;

    #[test]
    fn build_trade_plan_returns_watch_or_buy_with_full_plan_for_clean_turbo() {
        let signal = signal();
        let market = MarketContext {
            underlying_ticker: "RMS.PA".to_string(),
            spot: 1575.5,
            spot_currency: "EUR".to_string(),
            change_pct: 0.0,
            fx_rates: FxRateBook::default(),
            realized_volatility_20d: None,
            atr_14d: Some(30.0),
            support_1: None,
            support_2: None,
            resistance_1: Some(1620.5),
            resistance_2: None,
            risk_free_rate: 0.045,
            dividend_yield: 0.005,
        };

        let decision = build_trade_plan(&signal, &market, &DecisionConfig::default());
        let plan = decision.trade_plan.expect("plan should be computed");

        assert_eq!(plan.entry_price, 1.18);
        assert_eq!(plan.stop.underlying_stop_price, 1620.5);
        assert!(plan.target_1.underlying_target_price < market.spot);
        assert!(plan.target_2.gain_pct > plan.target_1.gain_pct);
        assert!(plan.position.suggested_notional_pct > 0.0);
    }

    #[test]
    fn last_unverified_price_cannot_be_entry() {
        let mut signal = signal();
        signal.execution_status = "last_unverified".to_string();
        signal.ask_price = None;

        assert_eq!(entry_price_for_long(&signal), None);
    }

    #[test]
    fn stop_projection_that_increases_product_price_is_adjusted_to_entry_breakeven() {
        let mut signal = signal();
        signal.ask_price = Some(0.50);
        signal.bid_price = Some(0.49);
        signal.last_price = 0.50;
        signal.mid_price = Some(0.495);
        signal.spread_pct = Some(2.0);
        let market = MarketContext {
            underlying_ticker: "RMS.PA".to_string(),
            spot: 1575.5,
            spot_currency: "EUR".to_string(),
            change_pct: 0.0,
            fx_rates: FxRateBook::default(),
            realized_volatility_20d: None,
            atr_14d: Some(30.0),
            support_1: None,
            support_2: None,
            resistance_1: Some(1620.5),
            resistance_2: None,
            risk_free_rate: 0.045,
            dividend_yield: 0.005,
        };

        let decision = build_trade_plan(&signal, &market, &DecisionConfig::default());

        let plan = decision.trade_plan.expect("discounted product should get adjusted stop");
        assert_eq!(plan.stop.stop_reason, crate::decision::models::StopReason::EntryBreakeven);
        assert!(plan.stop.loss_pct > 0.0);
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
