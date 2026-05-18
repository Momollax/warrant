use crate::indicators::warrant::OpportunitySignal;
use crate::models::fx::FxRateBook;

#[derive(Debug, Clone)]
pub struct MarketContext {
    pub underlying_ticker: String,
    pub spot: f64,
    pub spot_currency: String,
    pub change_pct: f64,
    pub fx_rates: FxRateBook,
    pub realized_volatility_20d: Option<f64>,
    pub atr_14d: Option<f64>,
    pub support_1: Option<f64>,
    pub support_2: Option<f64>,
    pub resistance_1: Option<f64>,
    pub resistance_2: Option<f64>,
    pub risk_free_rate: f64,
    pub dividend_yield: f64,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum DecisionAction {
    BuyCandidate,
    Watch,
    Avoid,
    ExitLoss,
    TakeProfit,
    Hold,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum StopReason {
    Atr,
    TechnicalSupportResistance,
    BarrierBuffer,
    EntryBreakeven,
    MaxLoss,
    FallbackPercent,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum TargetReason {
    RiskMultiple,
    TechnicalSupportResistance,
    IntrinsicRepricing,
    FallbackPercent,
}

#[derive(Debug, Clone)]
pub struct StopPlan {
    pub underlying_stop_price: f64,
    pub estimated_product_stop_price: f64,
    pub loss_pct: f64,
    pub distance_to_stop_pct: f64,
    pub stop_reason: StopReason,
}

#[derive(Debug, Clone)]
pub struct TargetPlan {
    pub underlying_target_price: f64,
    pub estimated_product_target_price: f64,
    pub gain_pct: f64,
    pub distance_to_target_pct: f64,
    pub target_reason: TargetReason,
}

#[derive(Debug, Clone)]
pub struct HorizonPlan {
    pub holding_days: u32,
    pub max_holding_days: u32,
    pub maturity_days: Option<i64>,
    pub theta_daily_pct: Option<f64>,
    pub theta_to_horizon_pct: Option<f64>,
    pub time_risk_score: f64,
}

#[derive(Debug, Clone)]
pub struct RiskPlan {
    pub barrier_distance_pct: Option<f64>,
    pub barrier_risk_score: Option<f64>,
    pub spread_pct: Option<f64>,
    pub spread_cost_underlying_pct: Option<f64>,
    pub data_quality_score: f64,
    pub liquidity_score: f64,
    pub max_loss_pct: f64,
}

#[derive(Debug, Clone)]
pub struct PositionPlan {
    pub account_risk_pct: f64,
    pub suggested_notional_pct: f64,
    pub max_notional_pct: f64,
    pub estimated_account_loss_pct: f64,
    pub products_per_1000_account: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct FeePlan {
    pub profile: String,
    pub order_notional: f64,
    pub buy_fee: f64,
    pub deposit_fee: f64,
    pub sell_stop_fee: f64,
    pub sell_target_1_fee: f64,
    pub sell_target_2_fee: f64,
    pub roundtrip_stop_fee_pct: f64,
    pub roundtrip_target_1_fee_pct: f64,
    pub roundtrip_target_2_fee_pct: f64,
    pub raw_stop_loss_pct: f64,
    pub raw_target_1_gain_pct: f64,
    pub raw_target_2_gain_pct: f64,
    pub net_stop_loss_pct: f64,
    pub net_target_1_gain_pct: f64,
    pub net_target_2_gain_pct: f64,
}

#[derive(Debug, Clone)]
pub struct TradePlan {
    pub entry_price: f64,
    pub entry_price_source: String,
    pub entry_underlying_price: f64,
    pub entry_edge_pct: f64,
    pub breakeven_move_pct: Option<f64>,
    pub stop: StopPlan,
    pub target_1: TargetPlan,
    pub target_2: TargetPlan,
    pub reward_risk_1: Option<f64>,
    pub reward_risk_2: Option<f64>,
    pub horizon: HorizonPlan,
    pub risk: RiskPlan,
    pub position: PositionPlan,
    pub fees: FeePlan,
    pub confidence_score: f64,
}

#[derive(Debug, Clone)]
pub struct DecisionSignal {
    pub opportunity: OpportunitySignal,
    pub trade_plan: Option<TradePlan>,
    pub decision: DecisionAction,
    pub reasons: Vec<String>,
    pub warnings: Vec<String>,
}
