use chrono::{DateTime, Utc};

/// Données nettoyées d'un warrant à un instant T.
#[derive(Debug)]
pub struct WarrantSnapshot {
    pub ticker:      String,
    pub name:        String,
    pub currency:    String,
    pub price:       f64,
    pub prev_close:  f64,
    pub change_pct:  f64,
    pub volume:      u64,
    pub high_52w:    f64,
    pub low_52w:     f64,
    pub last_candle: Option<Candle>,
    #[allow(dead_code)]
    pub fetched_at:  DateTime<Utc>,
}

/// Bougie OHLCV d'une période (1 minute par défaut).
#[derive(Debug)]
pub struct Candle {
    pub open:   f64,
    pub high:   f64,
    pub low:    f64,
    pub close:  f64,
    pub volume: u64,
}
