use crate::models::candle::{Candle, CandleSeries};

#[derive(Debug, Clone, Default)]
pub struct MarketIndicators {
    pub realized_volatility_20d: Option<f64>,
    pub atr_14d: Option<f64>,
    pub support_1: Option<f64>,
    pub support_2: Option<f64>,
    pub resistance_1: Option<f64>,
    pub resistance_2: Option<f64>,
}

pub fn compute_market_indicators(series: &CandleSeries) -> MarketIndicators {
    MarketIndicators {
        realized_volatility_20d: realized_volatility(series, 20),
        atr_14d: atr(series, 14),
        support_1: nth_recent_low(series, 1),
        support_2: nth_recent_low(series, 2),
        resistance_1: nth_recent_high(series, 1),
        resistance_2: nth_recent_high(series, 2),
    }
}

pub fn atr(series: &CandleSeries, period: usize) -> Option<f64> {
    if period == 0 || series.candles.len() < period + 1 {
        return None;
    }

    let start = series.candles.len().saturating_sub(period);
    let mut true_ranges = Vec::with_capacity(period);
    for index in start..series.candles.len() {
        let candle = &series.candles[index];
        let previous_close = series.candles.get(index.saturating_sub(1))?.close;
        let tr = (candle.high - candle.low)
            .max((candle.high - previous_close).abs())
            .max((candle.low - previous_close).abs());
        if tr.is_finite() && tr >= 0.0 {
            true_ranges.push(tr);
        }
    }

    average(&true_ranges)
}

pub fn realized_volatility(series: &CandleSeries, period: usize) -> Option<f64> {
    if period == 0 || series.candles.len() < period + 1 {
        return None;
    }

    let closes = series
        .candles
        .iter()
        .map(|candle| candle.close)
        .collect::<Vec<_>>();
    let start = closes.len().saturating_sub(period + 1);
    let returns = closes[start..]
        .windows(2)
        .filter_map(|window| {
            let previous = window[0];
            let current = window[1];
            (previous > 0.0 && current > 0.0).then_some((current / previous).ln())
        })
        .collect::<Vec<_>>();

    let std_dev = sample_std_dev(&returns)?;
    Some(std_dev * 252.0_f64.sqrt())
}

fn nth_recent_low(series: &CandleSeries, rank: usize) -> Option<f64> {
    nth_recent_extreme(series, rank, |candle| candle.low, true)
}

fn nth_recent_high(series: &CandleSeries, rank: usize) -> Option<f64> {
    nth_recent_extreme(series, rank, |candle| candle.high, false)
}

fn nth_recent_extreme(
    series: &CandleSeries,
    rank: usize,
    accessor: impl Fn(&Candle) -> f64,
    ascending: bool,
) -> Option<f64> {
    if rank == 0 {
        return None;
    }

    let mut values = series
        .candles
        .iter()
        .rev()
        .skip(1)
        .take(30)
        .map(accessor)
        .filter(|value| value.is_finite() && *value > 0.0)
        .collect::<Vec<_>>();
    if ascending {
        values.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    } else {
        values.sort_by(|left, right| right.partial_cmp(left).unwrap_or(std::cmp::Ordering::Equal));
    }
    values.dedup_by(|left, right| (*left - *right).abs() < 1e-9);
    values.get(rank - 1).copied()
}

fn average(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then_some(values.iter().sum::<f64>() / values.len() as f64)
}

fn sample_std_dev(values: &[f64]) -> Option<f64> {
    if values.len() < 2 {
        return None;
    }
    let mean = average(values)?;
    let variance =
        values.iter().map(|value| (value - mean).powi(2)).sum::<f64>() / (values.len() - 1) as f64;
    Some(variance.sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn atr_uses_true_range_against_previous_close() {
        let series = series(vec![
            candle(1, 10.0, 11.0, 9.0, 10.0),
            candle(2, 12.0, 14.0, 11.0, 13.0),
            candle(3, 13.0, 15.0, 12.0, 14.0),
        ]);

        let value = atr(&series, 2).unwrap();

        assert_close(value, 3.5, 1e-9);
    }

    #[test]
    fn support_and_resistance_ignore_current_candle() {
        let series = series(vec![
            candle(1, 10.0, 15.0, 8.0, 12.0),
            candle(2, 12.0, 16.0, 9.0, 14.0),
            candle(3, 14.0, 30.0, 1.0, 20.0),
        ]);
        let indicators = compute_market_indicators(&series);

        assert_eq!(indicators.support_1, Some(8.0));
        assert_eq!(indicators.resistance_1, Some(16.0));
    }

    #[test]
    fn realized_volatility_is_annualized() {
        let series = series(vec![
            candle(1, 10.0, 10.0, 10.0, 100.0),
            candle(2, 10.0, 10.0, 10.0, 101.0),
            candle(3, 10.0, 10.0, 10.0, 99.0),
            candle(4, 10.0, 10.0, 10.0, 102.0),
        ]);

        let value = realized_volatility(&series, 3).unwrap();

        assert!(value > 0.0);
    }

    fn series(candles: Vec<Candle>) -> CandleSeries {
        CandleSeries {
            ticker: "TEST".to_string(),
            currency: "EUR".to_string(),
            range: "1mo".to_string(),
            interval: "1d".to_string(),
            source: "unit".to_string(),
            fetched_at: Utc::now(),
            candles,
        }
    }

    fn candle(timestamp: i64, open: f64, high: f64, low: f64, close: f64) -> Candle {
        Candle {
            timestamp,
            open,
            high,
            low,
            close,
            volume: 0,
        }
    }

    fn assert_close(actual: f64, expected: f64, tolerance: f64) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "actual={actual}, expected={expected}, tolerance={tolerance}"
        );
    }
}
