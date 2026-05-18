use crate::decision::config::DecisionConfig;
use crate::decision::models::{DecisionAction, TradePlan};
use crate::indicators::warrant::OpportunitySignal;

pub fn compute_confidence_score(
    signal: &OpportunitySignal,
    reward_risk_2: Option<f64>,
    time_risk_score: f64,
    config: &DecisionConfig,
) -> f64 {
    let spread_score = signal
        .spread_pct
        .map(|spread| 100.0 - (spread / config.max_spread_pct * 100.0).clamp(0.0, 100.0))
        .unwrap_or(0.0);
    let peer_score = (signal.peer_count as f64 / 10.0 * 100.0).clamp(0.0, 100.0);
    let edge_score = (signal.spread_adjusted_gap_pct.max(0.0) / 5.0 * 100.0).clamp(0.0, 100.0);
    let reward_risk_score = reward_risk_2
        .map(|value| (value / config.min_reward_risk * 100.0).clamp(0.0, 100.0))
        .unwrap_or(0.0);
    let barrier_score = signal
        .barrier_distance_pct
        .map(|value| (value / config.min_barrier_distance_pct * 100.0).clamp(0.0, 100.0))
        .unwrap_or(50.0);
    let time_score = time_risk_score.clamp(0.0, 100.0);

    signal.data_quality_score * 0.25
        + signal.liquidity_score * 0.15
        + spread_score * 0.15
        + peer_score * 0.10
        + edge_score * 0.15
        + reward_risk_score * 0.10
        + barrier_score * 0.05
        + time_score * 0.05
}

pub fn decide_action(
    signal: &OpportunitySignal,
    plan: &TradePlan,
    config: &DecisionConfig,
) -> (DecisionAction, Vec<String>, Vec<String>) {
    let mut reasons = Vec::new();
    let mut warnings = Vec::new();

    if signal.execution_status != "executable_bid_ask" {
        reasons.push("bid_ask_not_executable".to_string());
    }
    if signal.data_quality_score < config.min_data_quality_score {
        reasons.push("data_quality_too_low".to_string());
    }
    if signal.liquidity_score < config.min_liquidity_score {
        reasons.push("liquidity_too_low".to_string());
    }
    if signal.spread_pct.unwrap_or(f64::INFINITY) > config.max_spread_pct {
        reasons.push("spread_too_wide".to_string());
    }
    if signal.peer_count < config.min_peer_count {
        reasons.push("not_enough_peers".to_string());
    }
    if signal
        .barrier_distance_pct
        .is_some_and(|value| value < config.min_barrier_distance_pct)
    {
        reasons.push("barrier_too_close".to_string());
    }
    if plan.stop.loss_pct > config.max_loss_pct_per_trade {
        reasons.push("stop_loss_too_large".to_string());
    }
    if plan
        .horizon
        .maturity_days
        .is_some_and(|days| days < config.min_maturity_days as i64)
    {
        reasons.push("maturity_too_short".to_string());
    }
    if let Some(theta_to_horizon) = plan.horizon.theta_to_horizon_pct {
        if theta_to_horizon > config.max_theta_to_horizon_pct * 2.0 {
            reasons.push("theta_cost_too_high".to_string());
        } else if theta_to_horizon > config.max_theta_to_horizon_pct {
            warnings.push("theta_cost_high".to_string());
        }
    }

    if let Some(rr) = plan.reward_risk_2 {
        if rr < config.min_reward_risk {
            warnings.push("reward_risk_below_threshold".to_string());
        }
    } else {
        reasons.push("reward_risk_missing".to_string());
    }

    if !reasons.is_empty() {
        return (DecisionAction::Avoid, reasons, warnings);
    }

    if signal.spread_adjusted_gap_pct >= config.min_entry_edge_pct
        && plan.confidence_score >= config.min_confidence_score
        && plan.reward_risk_2.unwrap_or(0.0) >= config.min_reward_risk
        && warnings.is_empty()
    {
        reasons.push("entry_conditions_met".to_string());
        (DecisionAction::BuyCandidate, reasons, warnings)
    } else {
        reasons.push("watch_conditions_only".to_string());
        (DecisionAction::Watch, reasons, warnings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::models::{
        FeePlan, HorizonPlan, PositionPlan, RiskPlan, StopPlan, StopReason, TargetPlan,
        TargetReason,
    };
    use crate::indicators::warrant::{OpportunitySignal, ValuationSide};

    #[test]
    fn clean_plan_can_be_buy_candidate() {
        let signal = signal(2.4, Some(0.1), 95.0, 70.0, 10, Some(16.9));
        let mut plan = plan();
        plan.confidence_score = compute_confidence_score(
            &signal,
            plan.reward_risk_2,
            plan.horizon.time_risk_score,
            &DecisionConfig::default(),
        );

        let (decision, reasons, warnings) =
            decide_action(&signal, &plan, &DecisionConfig::default());

        assert_eq!(decision, DecisionAction::BuyCandidate);
        assert!(reasons.contains(&"entry_conditions_met".to_string()));
        assert!(warnings.is_empty());
    }

    #[test]
    fn wide_spread_is_avoid_even_with_positive_edge() {
        let signal = signal(4.7, Some(8.85), 90.0, 70.0, 8, Some(14.0));
        let mut plan = plan();
        plan.confidence_score = compute_confidence_score(
            &signal,
            plan.reward_risk_2,
            plan.horizon.time_risk_score,
            &DecisionConfig::default(),
        );

        let (decision, reasons, _) = decide_action(&signal, &plan, &DecisionConfig::default());

        assert_eq!(decision, DecisionAction::Avoid);
        assert!(reasons.contains(&"spread_too_wide".to_string()));
    }

    #[test]
    fn poor_reward_risk_is_watch_not_buy_when_data_is_clean() {
        let signal = signal(2.4, Some(0.1), 95.0, 70.0, 10, Some(16.9));
        let mut plan = plan();
        plan.reward_risk_2 = Some(1.1);
        plan.confidence_score = compute_confidence_score(
            &signal,
            plan.reward_risk_2,
            plan.horizon.time_risk_score,
            &DecisionConfig::default(),
        );

        let (decision, _, warnings) = decide_action(&signal, &plan, &DecisionConfig::default());

        assert_eq!(decision, DecisionAction::Watch);
        assert!(warnings.contains(&"reward_risk_below_threshold".to_string()));
    }

    #[test]
    fn short_maturity_is_avoid() {
        let signal = signal(2.4, Some(0.1), 95.0, 70.0, 10, Some(16.9));
        let mut plan = plan();
        plan.horizon.maturity_days = Some(5);
        plan.confidence_score = compute_confidence_score(
            &signal,
            plan.reward_risk_2,
            plan.horizon.time_risk_score,
            &DecisionConfig::default(),
        );

        let (decision, reasons, _) = decide_action(&signal, &plan, &DecisionConfig::default());

        assert_eq!(decision, DecisionAction::Avoid);
        assert!(reasons.contains(&"maturity_too_short".to_string()));
    }

    #[test]
    fn high_theta_is_watch() {
        let signal = signal(2.4, Some(0.1), 95.0, 70.0, 10, Some(16.9));
        let mut plan = plan();
        plan.horizon.theta_to_horizon_pct = Some(20.0);
        plan.confidence_score = compute_confidence_score(
            &signal,
            plan.reward_risk_2,
            plan.horizon.time_risk_score,
            &DecisionConfig::default(),
        );

        let (decision, _, warnings) = decide_action(&signal, &plan, &DecisionConfig::default());

        assert_eq!(decision, DecisionAction::Watch);
        assert!(warnings.contains(&"theta_cost_high".to_string()));
    }

    fn signal(
        edge: f64,
        spread: Option<f64>,
        data_quality: f64,
        liquidity: f64,
        peer_count: usize,
        barrier_distance: Option<f64>,
    ) -> OpportunitySignal {
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
            last_price: 1.0,
            price_source: "boursorama_ask".to_string(),
            bid_price: Some(0.99),
            ask_price: Some(1.0),
            mid_price: Some(0.995),
            spread_pct: spread,
            bid_size: Some(100.0),
            ask_size: Some(100.0),
            quote_volume: Some(10.0),
            execution_status: "executable_bid_ask".to_string(),
            data_quality_score: data_quality,
            liquidity_score: liquidity,
            strike: 1800.0,
            strike_currency: "EUR".to_string(),
            barrier: Some(1800.0),
            barrier_currency: Some("EUR".to_string()),
            barrier_distance_pct: barrier_distance,
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
            peer_gap_pct: edge,
            spread_adjusted_gap_pct: edge,
            peer_count,
            valuation: ValuationSide::Undervalued,
            score: edge,
            note: String::new(),
        }
    }

    fn plan() -> TradePlan {
        TradePlan {
            entry_price: 1.0,
            entry_price_source: "boursorama_ask".to_string(),
            entry_underlying_price: 100.0,
            entry_edge_pct: 2.4,
            breakeven_move_pct: Some(0.02),
            stop: StopPlan {
                underlying_stop_price: 105.0,
                estimated_product_stop_price: 0.8,
                loss_pct: 20.0,
                distance_to_stop_pct: 5.0,
                stop_reason: StopReason::Atr,
            },
            target_1: TargetPlan {
                underlying_target_price: 92.5,
                estimated_product_target_price: 1.3,
                gain_pct: 30.0,
                distance_to_target_pct: 7.5,
                target_reason: TargetReason::RiskMultiple,
            },
            target_2: TargetPlan {
                underlying_target_price: 87.5,
                estimated_product_target_price: 1.6,
                gain_pct: 60.0,
                distance_to_target_pct: 12.5,
                target_reason: TargetReason::RiskMultiple,
            },
            reward_risk_1: Some(1.5),
            reward_risk_2: Some(3.0),
            horizon: HorizonPlan {
                holding_days: 21,
                max_holding_days: 35,
                maturity_days: None,
                theta_daily_pct: None,
                theta_to_horizon_pct: None,
                time_risk_score: 90.0,
            },
            risk: RiskPlan {
                barrier_distance_pct: Some(16.9),
                barrier_risk_score: Some(0.26),
                spread_pct: Some(0.1),
                spread_cost_underlying_pct: Some(0.02),
                data_quality_score: 95.0,
                liquidity_score: 70.0,
                max_loss_pct: 20.0,
            },
            position: PositionPlan {
                account_risk_pct: 1.0,
                suggested_notional_pct: 5.0,
                max_notional_pct: 10.0,
                estimated_account_loss_pct: 1.0,
                products_per_1000_account: Some(50.0),
            },
            fees: FeePlan {
                profile: "custom".to_string(),
                order_notional: 1000.0,
                buy_fee: 0.0,
                deposit_fee: 0.0,
                sell_stop_fee: 0.0,
                sell_target_1_fee: 0.0,
                sell_target_2_fee: 0.0,
                roundtrip_stop_fee_pct: 0.0,
                roundtrip_target_1_fee_pct: 0.0,
                roundtrip_target_2_fee_pct: 0.0,
                raw_stop_loss_pct: 20.0,
                raw_target_1_gain_pct: 30.0,
                raw_target_2_gain_pct: 60.0,
                net_stop_loss_pct: 20.0,
                net_target_1_gain_pct: 30.0,
                net_target_2_gain_pct: 60.0,
            },
            confidence_score: 0.0,
        }
    }
}
