#[derive(Debug, Clone)]
pub struct DecisionConfig {
    pub min_entry_edge_pct: f64,
    pub min_confidence_score: f64,
    pub max_spread_pct: f64,
    pub min_data_quality_score: f64,
    pub min_liquidity_score: f64,
    pub min_peer_count: usize,
    pub min_reward_risk: f64,
    pub max_loss_pct_per_trade: f64,
    pub account_risk_pct: f64,
    pub max_position_notional_pct: f64,
    pub default_holding_days: u32,
    pub max_holding_days: u32,
    pub stop_atr_multiple: f64,
    pub target_1_r_multiple: f64,
    pub target_2_r_multiple: f64,
    pub min_barrier_distance_pct: f64,
    pub barrier_stop_buffer_pct: f64,
    pub max_theta_to_horizon_pct: f64,
    pub min_maturity_days: u32,
    pub broker_fee_profile: String,
    pub fee_order_notional: f64,
    pub fee_buy_fixed: f64,
    pub fee_buy_pct: f64,
    pub fee_sell_fixed: f64,
    pub fee_sell_pct: f64,
    pub fee_deposit_fixed: f64,
    pub fee_deposit_pct: f64,
}

impl Default for DecisionConfig {
    fn default() -> Self {
        Self {
            min_entry_edge_pct: 1.0,
            min_confidence_score: 75.0,
            max_spread_pct: 5.0,
            min_data_quality_score: 80.0,
            min_liquidity_score: 50.0,
            min_peer_count: 5,
            min_reward_risk: 1.5,
            max_loss_pct_per_trade: 25.0,
            account_risk_pct: 1.0,
            max_position_notional_pct: 10.0,
            default_holding_days: 21,
            max_holding_days: 35,
            stop_atr_multiple: 1.5,
            target_1_r_multiple: 1.5,
            target_2_r_multiple: 2.5,
            min_barrier_distance_pct: 8.0,
            barrier_stop_buffer_pct: 2.0,
            max_theta_to_horizon_pct: 15.0,
            min_maturity_days: 21,
            broker_fee_profile: "custom".to_string(),
            fee_order_notional: 1000.0,
            fee_buy_fixed: 0.0,
            fee_buy_pct: 0.0,
            fee_sell_fixed: 0.0,
            fee_sell_pct: 0.0,
            fee_deposit_fixed: 0.0,
            fee_deposit_pct: 0.0,
        }
    }
}

impl DecisionConfig {
    pub fn from_env() -> Self {
        let default = Self::default();
        let profile_name = std::env::var("BROKER_FEE_PROFILE")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| default.broker_fee_profile.clone());
        let profile = fee_profile_defaults(&profile_name);
        Self {
            min_entry_edge_pct: read_env_f64("DECISION_MIN_ENTRY_EDGE_PCT")
                .unwrap_or(default.min_entry_edge_pct),
            min_confidence_score: read_env_f64("DECISION_MIN_CONFIDENCE_SCORE")
                .unwrap_or(default.min_confidence_score),
            max_spread_pct: read_env_f64("DECISION_MAX_SPREAD_PCT")
                .unwrap_or(default.max_spread_pct),
            min_data_quality_score: read_env_f64("DECISION_MIN_DATA_QUALITY_SCORE")
                .unwrap_or(default.min_data_quality_score),
            min_liquidity_score: read_env_f64("DECISION_MIN_LIQUIDITY_SCORE")
                .unwrap_or(default.min_liquidity_score),
            min_peer_count: read_env_usize("DECISION_MIN_PEER_COUNT")
                .unwrap_or(default.min_peer_count),
            min_reward_risk: read_env_f64("DECISION_MIN_REWARD_RISK")
                .unwrap_or(default.min_reward_risk),
            max_loss_pct_per_trade: read_env_f64("DECISION_MAX_LOSS_PCT_PER_TRADE")
                .unwrap_or(default.max_loss_pct_per_trade),
            account_risk_pct: read_env_f64("DECISION_ACCOUNT_RISK_PCT")
                .unwrap_or(default.account_risk_pct),
            max_position_notional_pct: read_env_f64("DECISION_MAX_POSITION_NOTIONAL_PCT")
                .unwrap_or(default.max_position_notional_pct),
            default_holding_days: read_env_u32("DECISION_DEFAULT_HOLDING_DAYS")
                .unwrap_or(default.default_holding_days),
            max_holding_days: read_env_u32("DECISION_MAX_HOLDING_DAYS")
                .unwrap_or(default.max_holding_days),
            stop_atr_multiple: read_env_f64("DECISION_STOP_ATR_MULTIPLE")
                .unwrap_or(default.stop_atr_multiple),
            target_1_r_multiple: read_env_f64("DECISION_TARGET_1_R_MULTIPLE")
                .unwrap_or(default.target_1_r_multiple),
            target_2_r_multiple: read_env_f64("DECISION_TARGET_2_R_MULTIPLE")
                .unwrap_or(default.target_2_r_multiple),
            min_barrier_distance_pct: read_env_f64("DECISION_MIN_BARRIER_DISTANCE_PCT")
                .unwrap_or(default.min_barrier_distance_pct),
            barrier_stop_buffer_pct: read_env_f64("DECISION_BARRIER_STOP_BUFFER_PCT")
                .unwrap_or(default.barrier_stop_buffer_pct),
            max_theta_to_horizon_pct: read_env_f64("DECISION_MAX_THETA_TO_HORIZON_PCT")
                .unwrap_or(default.max_theta_to_horizon_pct),
            min_maturity_days: read_env_u32("DECISION_MIN_MATURITY_DAYS")
                .unwrap_or(default.min_maturity_days),
            broker_fee_profile: profile_name,
            fee_order_notional: read_env_f64("FEE_ORDER_NOTIONAL")
                .unwrap_or(profile.fee_order_notional),
            fee_buy_fixed: read_env_f64("FEE_BUY_FIXED").unwrap_or(profile.fee_buy_fixed),
            fee_buy_pct: read_env_f64("FEE_BUY_PCT").unwrap_or(profile.fee_buy_pct),
            fee_sell_fixed: read_env_f64("FEE_SELL_FIXED").unwrap_or(profile.fee_sell_fixed),
            fee_sell_pct: read_env_f64("FEE_SELL_PCT").unwrap_or(profile.fee_sell_pct),
            fee_deposit_fixed: read_env_f64("FEE_DEPOSIT_FIXED")
                .unwrap_or(profile.fee_deposit_fixed),
            fee_deposit_pct: read_env_f64("FEE_DEPOSIT_PCT")
                .unwrap_or(profile.fee_deposit_pct),
        }
    }
}

fn fee_profile_defaults(profile: &str) -> DecisionConfig {
    let mut config = DecisionConfig::default();
    match normalize_profile(profile).as_str() {
        "trade_republic" => {
            config.fee_buy_fixed = 1.0;
            config.fee_sell_fixed = 1.0;
        }
        "bourse_direct_500" => {
            config.fee_buy_fixed = 0.99;
            config.fee_sell_fixed = 0.99;
        }
        "bourse_direct_1000" => {
            config.fee_buy_fixed = 1.90;
            config.fee_sell_fixed = 1.90;
        }
        "bourse_direct_2000" => {
            config.fee_buy_fixed = 2.90;
            config.fee_sell_fixed = 2.90;
        }
        "bourse_direct_pct" => {
            config.fee_buy_pct = 0.09;
            config.fee_sell_pct = 0.09;
        }
        "bourse_direct_morgan_stanley" => {}
        "degiro_fr_actions" => {
            config.fee_buy_fixed = 2.0;
            config.fee_sell_fixed = 2.0;
        }
        "degiro_otc_sg_bnp" => {
            config.fee_buy_fixed = 0.50;
            config.fee_sell_fixed = 0.50;
        }
        "fortuneo_starter" => {
            config.fee_buy_pct = 0.35;
            config.fee_sell_pct = 0.35;
        }
        "fortuneo_starter_first_500" => {}
        _ => {}
    }
    config
}

fn normalize_profile(profile: &str) -> String {
    profile
        .trim()
        .to_ascii_lowercase()
        .replace(['-', ' '], "_")
}

fn read_env_f64(name: &str) -> Option<f64> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite())
}

fn read_env_usize(name: &str) -> Option<usize> {
    std::env::var(name).ok().and_then(|value| value.parse().ok())
}

fn read_env_u32(name: &str) -> Option<u32> {
    std::env::var(name).ok().and_then(|value| value.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_conservative_entry_guards() {
        let config = DecisionConfig::default();

        assert!(config.min_entry_edge_pct > 0.0);
        assert!(config.max_spread_pct > 0.0);
        assert!(config.min_reward_risk >= 1.0);
        assert!(config.min_barrier_distance_pct > 0.0);
        assert_eq!(config.broker_fee_profile, "custom");
        assert!(config.fee_order_notional > 0.0);
    }
}
