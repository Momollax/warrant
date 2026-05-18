use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use reqwest::Client;

use crate::models::candle::{Candle, CandleSeries};
use crate::models::yahoo::YahooResponse;

pub async fn fetch_yahoo_candles(
    client: &Client,
    ticker: &str,
    range: &str,
    interval: &str,
) -> Result<CandleSeries> {
    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{ticker}?interval={interval}&range={range}&includePrePost=false"
    );

    let response = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("Erreur HTTP historique Yahoo pour {ticker}"))?;

    let status = response.status();
    if status == 404 {
        return Err(anyhow!("Ticker '{ticker}' introuvable chez Yahoo Finance"));
    }
    if !status.is_success() {
        return Err(anyhow!(
            "Yahoo Finance a repondu {} pour l'historique {ticker}",
            status
        ));
    }

    let raw: YahooResponse = response
        .json()
        .await
        .with_context(|| format!("Impossible de parser l'historique JSON pour {ticker}"))?;

    if let Some(err) = &raw.chart.error {
        return Err(anyhow!("Erreur Yahoo historique pour {ticker}: {err}"));
    }

    let result = raw
        .chart
        .result
        .and_then(|mut results| results.pop())
        .ok_or_else(|| anyhow!("Aucun historique Yahoo pour {ticker}"))?;

    let currency = result
        .meta
        .currency
        .clone()
        .unwrap_or_else(|| "UNKNOWN".to_string());
    let candles = parse_candles(&result.timestamp.unwrap_or_default(), &result.indicators.quote);

    Ok(CandleSeries {
        ticker: result.meta.symbol,
        currency,
        range: range.to_string(),
        interval: interval.to_string(),
        source: "yahoo_chart".to_string(),
        fetched_at: Utc::now(),
        candles,
    })
}

fn parse_candles(
    timestamps: &[i64],
    quotes: &Option<Vec<crate::models::yahoo::Quote>>,
) -> Vec<Candle> {
    let Some(quote) = quotes.as_ref().and_then(|quotes| quotes.first()) else {
        return Vec::new();
    };

    timestamps
        .iter()
        .enumerate()
        .filter_map(|(index, timestamp)| {
            let open = value_at(&quote.open, index)?;
            let high = value_at(&quote.high, index)?;
            let low = value_at(&quote.low, index)?;
            let close = value_at(&quote.close, index)?;
            let volume = quote
                .volume
                .as_ref()
                .and_then(|values| values.get(index))
                .copied()
                .flatten()
                .unwrap_or(0);

            Some(Candle {
                timestamp: *timestamp,
                open,
                high,
                low,
                close,
                volume,
            })
        })
        .collect()
}

fn value_at(values: &Option<Vec<Option<f64>>>, index: usize) -> Option<f64> {
    values
        .as_ref()
        .and_then(|values| values.get(index))
        .copied()
        .flatten()
        .filter(|value| value.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::yahoo::Quote;

    #[test]
    fn parse_candles_keeps_only_complete_ohlc_rows() {
        let timestamps = vec![1, 2, 3];
        let quotes = Some(vec![Quote {
            open: Some(vec![Some(10.0), None, Some(12.0)]),
            high: Some(vec![Some(11.0), Some(12.0), Some(13.0)]),
            low: Some(vec![Some(9.0), Some(10.0), Some(11.0)]),
            close: Some(vec![Some(10.5), Some(11.0), Some(12.5)]),
            volume: Some(vec![Some(100), Some(200), None]),
        }]);

        let candles = parse_candles(&timestamps, &quotes);

        assert_eq!(candles.len(), 2);
        assert_eq!(candles[0].timestamp, 1);
        assert_eq!(candles[0].volume, 100);
        assert_eq!(candles[1].timestamp, 3);
        assert_eq!(candles[1].volume, 0);
    }
}
