use crate::decision::config::DecisionConfig;
use crate::decision::models::{FeePlan, StopPlan, TargetPlan};

pub fn compute_fee_plan(
    entry_price: f64,
    stop: &StopPlan,
    target_1: &TargetPlan,
    target_2: &TargetPlan,
    config: &DecisionConfig,
) -> Option<FeePlan> {
    if !entry_price.is_finite() || entry_price <= 0.0 || !config.fee_order_notional.is_finite() {
        return None;
    }
    let order_notional = config.fee_order_notional.max(0.0);
    if order_notional <= 0.0 {
        return None;
    }

    let stop_notional =
        exit_notional(order_notional, entry_price, stop.estimated_product_stop_price)?;
    let target_1_notional =
        exit_notional(order_notional, entry_price, target_1.estimated_product_target_price)?;
    let target_2_notional =
        exit_notional(order_notional, entry_price, target_2.estimated_product_target_price)?;

    let buy_fee = order_fee(order_notional, config.fee_buy_fixed, config.fee_buy_pct)?;
    let deposit_fee = order_fee(
        order_notional,
        config.fee_deposit_fixed,
        config.fee_deposit_pct,
    )?;
    let sell_stop_fee = order_fee(stop_notional, config.fee_sell_fixed, config.fee_sell_pct)?;
    let sell_target_1_fee =
        order_fee(target_1_notional, config.fee_sell_fixed, config.fee_sell_pct)?;
    let sell_target_2_fee =
        order_fee(target_2_notional, config.fee_sell_fixed, config.fee_sell_pct)?;

    let cost_basis = order_notional + buy_fee + deposit_fee;
    if cost_basis <= 0.0 {
        return None;
    }

    let net_stop_loss_pct = (cost_basis - (stop_notional - sell_stop_fee)) / cost_basis * 100.0;
    let net_target_1_gain_pct =
        ((target_1_notional - sell_target_1_fee) - cost_basis) / cost_basis * 100.0;
    let net_target_2_gain_pct =
        ((target_2_notional - sell_target_2_fee) - cost_basis) / cost_basis * 100.0;

    Some(FeePlan {
        profile: config.broker_fee_profile.clone(),
        order_notional,
        buy_fee,
        deposit_fee,
        sell_stop_fee,
        sell_target_1_fee,
        sell_target_2_fee,
        roundtrip_stop_fee_pct: fee_drag_pct(order_notional, buy_fee, deposit_fee, sell_stop_fee),
        roundtrip_target_1_fee_pct: fee_drag_pct(
            order_notional,
            buy_fee,
            deposit_fee,
            sell_target_1_fee,
        ),
        roundtrip_target_2_fee_pct: fee_drag_pct(
            order_notional,
            buy_fee,
            deposit_fee,
            sell_target_2_fee,
        ),
        raw_stop_loss_pct: stop.loss_pct,
        raw_target_1_gain_pct: target_1.gain_pct,
        raw_target_2_gain_pct: target_2.gain_pct,
        net_stop_loss_pct,
        net_target_1_gain_pct,
        net_target_2_gain_pct,
    })
}

fn exit_notional(entry_notional: f64, entry_price: f64, exit_price: f64) -> Option<f64> {
    (exit_price.is_finite() && exit_price >= 0.0)
        .then_some(entry_notional * exit_price / entry_price)
}

fn order_fee(notional: f64, fixed: f64, pct: f64) -> Option<f64> {
    if !notional.is_finite()
        || !fixed.is_finite()
        || !pct.is_finite()
        || fixed < 0.0
        || pct < 0.0
    {
        return None;
    }
    Some(fixed + notional * pct / 100.0)
}

fn fee_drag_pct(order_notional: f64, buy_fee: f64, deposit_fee: f64, sell_fee: f64) -> f64 {
    (buy_fee + deposit_fee + sell_fee) / order_notional * 100.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::models::{StopReason, TargetReason};

    #[test]
    fn fixed_buy_sell_fees_reduce_target_gain_and_increase_stop_loss() {
        let config = DecisionConfig {
            broker_fee_profile: "unit_test".to_string(),
            fee_order_notional: 1000.0,
            fee_buy_fixed: 1.0,
            fee_sell_fixed: 1.0,
            ..DecisionConfig::default()
        };
        let stop = StopPlan {
            underlying_stop_price: 95.0,
            estimated_product_stop_price: 9.0,
            loss_pct: 10.0,
            distance_to_stop_pct: 5.0,
            stop_reason: StopReason::Atr,
        };
        let target_1 = TargetPlan {
            underlying_target_price: 110.0,
            estimated_product_target_price: 11.0,
            gain_pct: 10.0,
            distance_to_target_pct: 10.0,
            target_reason: TargetReason::RiskMultiple,
        };
        let target_2 = TargetPlan {
            underlying_target_price: 120.0,
            estimated_product_target_price: 12.0,
            gain_pct: 20.0,
            distance_to_target_pct: 20.0,
            target_reason: TargetReason::RiskMultiple,
        };

        let fees = compute_fee_plan(10.0, &stop, &target_1, &target_2, &config).unwrap();

        assert_close(fees.net_stop_loss_pct, 10.1898101898, 1e-9);
        assert_close(fees.net_target_2_gain_pct, 19.7802197802, 1e-9);
        assert_close(fees.roundtrip_target_2_fee_pct, 0.2, 1e-9);
    }

    #[test]
    fn percentage_deposit_fee_is_included_in_cost_basis() {
        let config = DecisionConfig {
            fee_order_notional: 1000.0,
            fee_buy_pct: 0.35,
            fee_sell_pct: 0.35,
            fee_deposit_pct: 0.50,
            ..DecisionConfig::default()
        };
        let stop = StopPlan {
            underlying_stop_price: 95.0,
            estimated_product_stop_price: 9.5,
            loss_pct: 5.0,
            distance_to_stop_pct: 5.0,
            stop_reason: StopReason::Atr,
        };
        let target = TargetPlan {
            underlying_target_price: 105.0,
            estimated_product_target_price: 10.5,
            gain_pct: 5.0,
            distance_to_target_pct: 5.0,
            target_reason: TargetReason::RiskMultiple,
        };

        let fees = compute_fee_plan(10.0, &stop, &target, &target, &config).unwrap();

        assert!(fees.net_target_2_gain_pct < fees.raw_target_2_gain_pct);
        assert!(fees.net_stop_loss_pct > fees.raw_stop_loss_pct);
        assert_close(fees.deposit_fee, 5.0, 1e-9);
    }

    #[test]
    fn mixed_fixed_and_percentage_fees_match_manual_cashflows() {
        let config = DecisionConfig {
            broker_fee_profile: "manual".to_string(),
            fee_order_notional: 1000.0,
            fee_buy_fixed: 1.90,
            fee_buy_pct: 0.10,
            fee_sell_fixed: 1.90,
            fee_sell_pct: 0.20,
            fee_deposit_fixed: 2.00,
            fee_deposit_pct: 0.30,
            ..DecisionConfig::default()
        };
        let stop = StopPlan {
            underlying_stop_price: 90.0,
            estimated_product_stop_price: 4.50,
            loss_pct: 10.0,
            distance_to_stop_pct: 10.0,
            stop_reason: StopReason::Atr,
        };
        let target_1 = TargetPlan {
            underlying_target_price: 105.0,
            estimated_product_target_price: 5.50,
            gain_pct: 10.0,
            distance_to_target_pct: 5.0,
            target_reason: TargetReason::RiskMultiple,
        };
        let target_2 = TargetPlan {
            underlying_target_price: 110.0,
            estimated_product_target_price: 6.00,
            gain_pct: 20.0,
            distance_to_target_pct: 10.0,
            target_reason: TargetReason::RiskMultiple,
        };

        let fees = compute_fee_plan(5.0, &stop, &target_1, &target_2, &config).unwrap();
        let buy_fee = 1.90 + 1000.0 * 0.10 / 100.0;
        let deposit_fee = 2.00 + 1000.0 * 0.30 / 100.0;
        let target_2_notional = 1000.0 * 6.00 / 5.0;
        let sell_target_2_fee = 1.90 + target_2_notional * 0.20 / 100.0;
        let cost_basis = 1000.0 + buy_fee + deposit_fee;
        let expected_target_2 =
            ((target_2_notional - sell_target_2_fee) - cost_basis) / cost_basis * 100.0;

        assert_close(fees.buy_fee, buy_fee, 1e-12);
        assert_close(fees.deposit_fee, deposit_fee, 1e-12);
        assert_close(fees.sell_target_2_fee, sell_target_2_fee, 1e-12);
        assert_close(fees.net_target_2_gain_pct, expected_target_2, 1e-12);
    }

    fn assert_close(actual: f64, expected: f64, tolerance: f64) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "actual={actual}, expected={expected}, tolerance={tolerance}"
        );
    }
}
