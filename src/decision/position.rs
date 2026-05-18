use crate::decision::config::DecisionConfig;
use crate::decision::models::PositionPlan;

pub fn compute_position_plan(
    entry_price: f64,
    stop_loss_pct: f64,
    config: &DecisionConfig,
) -> Option<PositionPlan> {
    if !entry_price.is_finite()
        || entry_price <= 0.0
        || !stop_loss_pct.is_finite()
        || stop_loss_pct <= 0.0
        || !config.account_risk_pct.is_finite()
        || config.account_risk_pct <= 0.0
    {
        return None;
    }

    let uncapped_notional_pct = config.account_risk_pct / stop_loss_pct * 100.0;
    let max_notional_pct = config.max_position_notional_pct.max(0.0);
    let suggested_notional_pct = uncapped_notional_pct.min(max_notional_pct);
    let estimated_account_loss_pct = suggested_notional_pct * stop_loss_pct / 100.0;
    let products_per_1000_account = (suggested_notional_pct > 0.0)
        .then_some((1000.0 * suggested_notional_pct / 100.0) / entry_price);

    Some(PositionPlan {
        account_risk_pct: config.account_risk_pct,
        suggested_notional_pct,
        max_notional_pct,
        estimated_account_loss_pct,
        products_per_1000_account,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_position_from_account_risk_and_stop_loss() {
        let config = DecisionConfig::default();

        let plan = compute_position_plan(2.0, 20.0, &config).unwrap();

        assert_close(plan.suggested_notional_pct, 5.0, 1e-9);
        assert_close(plan.estimated_account_loss_pct, 1.0, 1e-9);
        assert_close(plan.products_per_1000_account.unwrap(), 25.0, 1e-9);
    }

    #[test]
    fn caps_position_notional() {
        let config = DecisionConfig {
            max_position_notional_pct: 3.0,
            ..DecisionConfig::default()
        };

        let plan = compute_position_plan(1.0, 10.0, &config).unwrap();

        assert_close(plan.suggested_notional_pct, 3.0, 1e-9);
        assert_close(plan.estimated_account_loss_pct, 0.3, 1e-9);
    }

    fn assert_close(actual: f64, expected: f64, tolerance: f64) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "actual={actual}, expected={expected}, tolerance={tolerance}"
        );
    }
}
