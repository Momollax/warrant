#[derive(Debug, Clone)]
pub struct FxRate {
    pub from: String,
    pub to: String,
    pub rate: f64,
    pub source_ticker: String,
}

#[derive(Debug, Clone, Default)]
pub struct FxRateBook {
    rates: Vec<FxRate>,
}

impl FxRateBook {
    pub fn add(&mut self, rate: FxRate) {
        if rate.rate.is_finite() && rate.rate > 0.0 {
            self.rates.push(rate);
        }
    }

    pub fn rate(&self, from: &str, to: &str) -> Option<f64> {
        if same_currency(from, to) {
            return Some(1.0);
        }

        self.rates
            .iter()
            .find(|rate| same_currency(&rate.from, from) && same_currency(&rate.to, to))
            .map(|rate| rate.rate)
            .or_else(|| {
                self.rates
                    .iter()
                    .find(|rate| same_currency(&rate.from, to) && same_currency(&rate.to, from))
                    .map(|rate| 1.0 / rate.rate)
            })
    }

    pub fn source_ticker(&self, from: &str, to: &str) -> Option<&str> {
        if same_currency(from, to) {
            return Some("IDENTITY");
        }

        self.rates
            .iter()
            .find(|rate| {
                (same_currency(&rate.from, from) && same_currency(&rate.to, to))
                    || (same_currency(&rate.from, to) && same_currency(&rate.to, from))
            })
            .map(|rate| rate.source_ticker.as_str())
    }

    pub fn convert(&self, amount: f64, from: &str, to: &str) -> Option<f64> {
        self.rate(from, to).map(|rate| amount * rate)
    }

    pub fn is_empty(&self) -> bool {
        self.rates.is_empty()
    }
}

fn same_currency(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_identity_direct_and_inverse_rates() {
        let mut book = FxRateBook::default();
        book.add(FxRate {
            from: "USD".to_string(),
            to: "EUR".to_string(),
            rate: 0.86,
            source_ticker: "EURUSD=X".to_string(),
        });

        assert_eq!(book.convert(10.0, "EUR", "EUR"), Some(10.0));
        assert_eq!(book.convert(100.0, "USD", "EUR"), Some(86.0));

        let eur_to_usd = book.convert(86.0, "EUR", "USD").unwrap();
        assert!((eur_to_usd - 100.0).abs() < 1e-10);
        assert_eq!(book.source_ticker("EUR", "USD"), Some("EURUSD=X"));
    }

    #[test]
    fn ignores_invalid_rates() {
        let mut book = FxRateBook::default();
        book.add(FxRate {
            from: "USD".to_string(),
            to: "EUR".to_string(),
            rate: 0.0,
            source_ticker: "BAD".to_string(),
        });
        book.add(FxRate {
            from: "GBP".to_string(),
            to: "EUR".to_string(),
            rate: f64::NAN,
            source_ticker: "BAD".to_string(),
        });

        assert!(book.is_empty());
        assert_eq!(book.convert(100.0, "USD", "EUR"), None);
    }

    #[test]
    fn currency_codes_are_case_insensitive() {
        let mut book = FxRateBook::default();
        book.add(FxRate {
            from: "usd".to_string(),
            to: "eur".to_string(),
            rate: 0.86,
            source_ticker: "EURUSD=X".to_string(),
        });

        assert_eq!(book.rate("USD", "eur"), Some(0.86));
        assert_close(book.convert(86.0, "EUR", "usd").unwrap(), 100.0, 1e-10);
        assert_eq!(book.source_ticker("Usd", "EUR"), Some("EURUSD=X"));
        assert_eq!(book.source_ticker("eur", "EUR"), Some("IDENTITY"));
    }

    fn assert_close(actual: f64, expected: f64, tolerance: f64) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "actual={actual}, expected={expected}, tolerance={tolerance}"
        );
    }
}
