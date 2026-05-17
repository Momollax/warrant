use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct EuronextDirectoryResponse {
    #[serde(rename = "iTotalDisplayRecords")]
    pub total_display_records: usize,
    #[serde(rename = "aaData")]
    pub rows: Vec<Vec<String>>,
}

#[derive(Debug)]
pub struct EuronextProduct {
    pub symbol: String,
    pub yahoo_symbol: Option<String>,
    pub isin: Option<String>,
    pub mic: Option<String>,
    pub name: Option<String>,
    pub underlying: String,
    pub product_type: String,
    pub strike: String,
    pub maturity: String,
    pub bid_ask: String,
    pub last_price: String,
    pub last_trade_time: String,
    pub detail_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct InstrumentDetailResponse {
    pub instr: InstrumentDetail,
}

#[derive(Debug, Deserialize)]
pub struct InstrumentDetail {
    #[serde(rename = "shrtNm")]
    pub short_name: Option<String>,
    #[serde(rename = "longNm")]
    pub long_name: Option<String>,
    pub currency: Option<String>,
    #[serde(rename = "issueDt")]
    pub issue_date: Option<String>,
    #[serde(rename = "issuePx")]
    pub issue_price: Option<String>,
    #[serde(rename = "issuePxCur")]
    pub issue_price_currency: Option<String>,
    #[serde(rename = "introDt")]
    pub introduction_date: Option<String>,
    #[serde(rename = "issuerName")]
    pub issuer_name: Option<String>,
    #[serde(rename = "parityWarUnder")]
    pub parity_warrant_underlying: Option<String>,
    #[serde(rename = "parityUnderWar")]
    pub parity_underlying_warrant: Option<String>,
    #[serde(rename = "strikePx")]
    pub strike_px: Option<String>,
    #[serde(rename = "strikePxCur")]
    pub strike_currency: Option<String>,
    #[serde(rename = "secondStrikePx")]
    pub second_strike_px: Option<String>,
    #[serde(rename = "secondStrikePxCur")]
    pub second_strike_currency: Option<String>,
    #[serde(rename = "tradLot")]
    pub trading_lot: Option<String>,
    #[serde(rename = "nbShare")]
    pub number_of_shares: Option<String>,
    #[serde(rename = "priceMultiplier")]
    pub price_multiplier: Option<String>,
    pub quality: Option<String>,
    pub listed: Option<String>,
    pub mic: Option<String>,
    #[serde(rename = "currInstrSess")]
    pub current_session: Option<InstrumentSession>,
    #[serde(rename = "prevInstrSess")]
    pub previous_session: Option<InstrumentSession>,
    #[serde(rename = "specFeat")]
    pub spec_features: Option<Vec<InstrumentSpecFeature>>,
    pub perf: Option<Vec<InstrumentPerformance>>,
    #[serde(rename = "tradingGroup")]
    pub trading_group: Option<TradingGroup>,
}

#[derive(Debug, Deserialize)]
pub struct InstrumentSession {
    #[serde(rename = "quotationState")]
    pub quotation_state: Option<String>,
    #[serde(rename = "instrTradingStatus")]
    pub trading_status: Option<String>,
    #[serde(rename = "lastPx")]
    pub last_price: Option<String>,
    #[serde(rename = "lastQty")]
    pub last_quantity: Option<String>,
    #[serde(rename = "openPx")]
    pub open_price: Option<String>,
    #[serde(rename = "openDtTm")]
    pub open_datetime: Option<String>,
    #[serde(rename = "closPx")]
    pub close_price: Option<String>,
    #[serde(rename = "indiClosPx")]
    pub indicative_close_price: Option<String>,
    #[serde(rename = "prevAdjClosingPrice")]
    pub previous_adjusted_close_price: Option<String>,
    #[serde(rename = "prevAdjClosingDateTime")]
    pub previous_adjusted_close_datetime: Option<String>,
    #[serde(rename = "tradedQty")]
    pub traded_quantity: Option<String>,
    #[serde(rename = "nbTrades")]
    pub trade_count: Option<String>,
    #[serde(rename = "valorization")]
    pub valorization: Option<String>,
    #[serde(rename = "valorizationDateTime")]
    pub valorization_datetime: Option<String>,
    #[serde(rename = "marketCapitalisation")]
    pub market_capitalisation: Option<String>,
    #[serde(rename = "exchangedCapitalisation")]
    pub exchanged_capitalisation: Option<String>,
    #[serde(rename = "lastUpdate")]
    pub last_update: Option<String>,
    #[serde(rename = "lastTradeType")]
    pub last_trade_type: Option<String>,
    #[serde(rename = "haltReason")]
    pub halt_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct InstrumentSpecFeature {
    pub field: String,
    pub value: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct InstrumentPerformance {
    #[serde(rename = "perType")]
    pub period_type: Option<String>,
    #[serde(rename = "highPx")]
    pub high_price: Option<String>,
    #[serde(rename = "lowPx")]
    pub low_price: Option<String>,
    #[serde(rename = "tradedQty")]
    pub traded_quantity: Option<String>,
    #[serde(rename = "tradedAmt")]
    pub traded_amount: Option<String>,
    #[serde(rename = "nbOpenDays")]
    pub open_days: Option<String>,
    #[serde(rename = "lastQuotDtTm")]
    pub last_quote_datetime: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TradingGroup {
    #[serde(rename = "tradingGroupCode")]
    pub trading_group_code: Option<String>,
    #[serde(rename = "tradingMode")]
    pub trading_mode: Option<String>,
    #[serde(rename = "timeOpening1")]
    pub time_opening_1: Option<String>,
    #[serde(rename = "timeClosing1")]
    pub time_closing_1: Option<String>,
    #[serde(rename = "timeOpening2")]
    pub time_opening_2: Option<String>,
    #[serde(rename = "timeClosing2")]
    pub time_closing_2: Option<String>,
    #[serde(rename = "eodTime")]
    pub eod_time: Option<String>,
}
