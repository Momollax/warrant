use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use reqwest::Client;

use crate::api::candles::fetch_yahoo_candles;
use crate::models::candle::CandleSeries;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum CacheMode {
    UseCache,
    Refresh,
}

impl CacheMode {
    pub fn from_str(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "refresh" | "force" | "reload" | "1" | "true" | "yes" | "on" => Self::Refresh,
            _ => Self::UseCache,
        }
    }
}

pub async fn load_or_fetch_candles(
    client: &Client,
    ticker: &str,
    range: &str,
    interval: &str,
    cache_dir: &Path,
    mode: CacheMode,
) -> Result<CandleSeries> {
    let path = candle_cache_path(cache_dir, ticker, range, interval);
    if mode == CacheMode::UseCache && path.exists() {
        return read_candle_cache(&path);
    }

    let series = fetch_yahoo_candles(client, ticker, range, interval).await?;
    write_candle_cache(&path, &series)?;
    Ok(series)
}

pub fn read_candle_cache(path: &Path) -> Result<CandleSeries> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("Impossible de lire le cache bougies {}", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("Cache bougies invalide {}", path.display()))
}

pub fn write_candle_cache(path: &Path, series: &CandleSeries) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Impossible de creer le dossier de cache bougies {}",
                parent.display()
            )
        })?;
    }
    let raw = serde_json::to_string_pretty(series)?;
    fs::write(path, raw)
        .with_context(|| format!("Impossible d'ecrire le cache bougies {}", path.display()))
}

pub fn candle_cache_path(cache_dir: &Path, ticker: &str, range: &str, interval: &str) -> PathBuf {
    let filename = format!(
        "{}__{}__{}.json",
        sanitize_component(ticker),
        sanitize_component(range),
        sanitize_component(interval)
    );
    cache_dir.join(filename)
}

fn sanitize_component(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();

    if sanitized.is_empty() {
        "unknown".to_string()
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    use crate::models::candle::Candle;

    #[test]
    fn cache_path_sanitizes_unsafe_components() {
        let path = candle_cache_path(Path::new("data/cache/candles"), "RMS/PA", "6mo", "1d");

        assert_eq!(
            path,
            Path::new("data/cache/candles").join("RMS_PA__6mo__1d.json")
        );
    }

    #[test]
    fn candle_cache_round_trips_json() {
        let root = std::env::temp_dir().join(format!(
            "warrant-candle-cache-test-{}",
            std::process::id()
        ));
        let path = candle_cache_path(&root, "RMS.PA", "6mo", "1d");
        let series = CandleSeries {
            ticker: "RMS.PA".to_string(),
            currency: "EUR".to_string(),
            range: "6mo".to_string(),
            interval: "1d".to_string(),
            source: "unit_test".to_string(),
            fetched_at: Utc::now(),
            candles: vec![Candle {
                timestamp: 1,
                open: 10.0,
                high: 11.0,
                low: 9.0,
                close: 10.5,
                volume: 100,
            }],
        };

        write_candle_cache(&path, &series).unwrap();
        let loaded = read_candle_cache(&path).unwrap();

        assert_eq!(loaded, series);

        let _ = fs::remove_dir_all(root);
    }
}
