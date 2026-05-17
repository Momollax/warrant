use crate::models::yahoo::Indicators;
use crate::models::warrant::Candle;

/// Extrait la dernière bougie complète depuis les indicateurs Yahoo Finance.
pub fn extract_last_candle(indicators: &Indicators) -> Option<Candle> {
    let quotes = indicators.quote.as_ref()?.first()?;

    let last = |v: &Option<Vec<Option<f64>>>| -> Option<f64> {
        v.as_ref()?.iter().rev().find_map(|x| *x)
    };
    let last_vol = |v: &Option<Vec<Option<u64>>>| -> Option<u64> {
        v.as_ref()?.iter().rev().find_map(|x| *x)
    };

    Some(Candle {
        open:   last(&quotes.open)?,
        high:   last(&quotes.high)?,
        low:    last(&quotes.low)?,
        close:  last(&quotes.close)?,
        volume: last_vol(&quotes.volume).unwrap_or(0),
    })
}
