use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Candle {
    pub timestamp: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CandleSeries {
    pub ticker: String,
    pub currency: String,
    pub range: String,
    pub interval: String,
    pub source: String,
    pub fetched_at: DateTime<Utc>,
    pub candles: Vec<Candle>,
}

impl CandleSeries {
    pub fn is_empty(&self) -> bool {
        self.candles.is_empty()
    }

    pub fn last_close(&self) -> Option<f64> {
        self.candles.last().map(|candle| candle.close)
    }
}
