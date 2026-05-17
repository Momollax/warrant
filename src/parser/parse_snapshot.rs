use anyhow::{anyhow, Result};
use chrono::Utc;

use crate::models::yahoo::ChartResult;
use crate::models::warrant::WarrantSnapshot;
use crate::parser::extract_candle::extract_last_candle;

/// Transforme une réponse brute Yahoo Finance en WarrantSnapshot utilisable.
pub fn parse_snapshot(r: ChartResult) -> Result<WarrantSnapshot> {
    let meta = &r.meta;

    let price = meta
        .regular_market_price
        .ok_or_else(|| anyhow!("Prix manquant pour {}", meta.symbol))?;

    let prev_close = meta.previous_close.unwrap_or(price);
    let change_pct = if prev_close != 0.0 {
        (price - prev_close) / prev_close * 100.0
    } else {
        0.0
    };

    let last_candle = extract_last_candle(&r.indicators);

    Ok(WarrantSnapshot {
        ticker:      meta.symbol.clone(),
        name:        meta.short_name.clone().unwrap_or_else(|| meta.symbol.clone()),
        currency:    meta.currency.clone().unwrap_or_else(|| "EUR".into()),
        price,
        prev_close,
        change_pct,
        volume:      meta.regular_market_volume.unwrap_or(0),
        high_52w:    meta.fifty_two_week_high.unwrap_or(0.0),
        low_52w:     meta.fifty_two_week_low.unwrap_or(0.0),
        last_candle,
        fetched_at:  Utc::now(),
    })
}
