use serde::Deserialize;

/// Réponse brute de l'API Yahoo Finance chart.
#[derive(Debug, Deserialize)]
pub struct YahooResponse {
    pub chart: Chart,
}

#[derive(Debug, Deserialize)]
pub struct Chart {
    pub result: Option<Vec<ChartResult>>,
    pub error:  Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct ChartResult {
    pub meta:              Meta,
    #[allow(dead_code)]
    pub timestamp:         Option<Vec<i64>>,
    pub indicators:        Indicators,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Meta {
    pub symbol:   String,
    pub currency: Option<String>,
    #[serde(rename = "regularMarketPrice")]
    pub regular_market_price: Option<f64>,
    #[serde(rename = "regularMarketTime")]
    #[allow(dead_code)]
    pub regular_market_time: Option<i64>,
    #[serde(rename = "regularMarketVolume")]
    pub regular_market_volume: Option<u64>,
    #[serde(rename = "fiftyTwoWeekHigh")]
    pub fifty_two_week_high: Option<f64>,
    #[serde(rename = "fiftyTwoWeekLow")]
    pub fifty_two_week_low: Option<f64>,
    #[serde(rename = "previousClose")]
    pub previous_close: Option<f64>,
    #[serde(rename = "shortName")]
    pub short_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Indicators {
    pub quote: Option<Vec<Quote>>,
}

#[derive(Debug, Deserialize)]
pub struct Quote {
    pub open:   Option<Vec<Option<f64>>>,
    pub high:   Option<Vec<Option<f64>>>,
    pub low:    Option<Vec<Option<f64>>>,
    pub close:  Option<Vec<Option<f64>>>,
    pub volume: Option<Vec<Option<u64>>>,
}
