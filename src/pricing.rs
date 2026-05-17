use crate::models::structured::StructuredProduct;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ProductFamily {
    Warrant,
    OpenEndKnockOut,
    MiniFuture,
    Turbo,
    Certificate,
    Other,
}

impl ProductFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Warrant => "warrant",
            Self::OpenEndKnockOut => "open_end_knock_out",
            Self::MiniFuture => "mini_future",
            Self::Turbo => "turbo",
            Self::Certificate => "certificate",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PricingModel {
    WarrantIntrinsic,
    FinancingLevel,
    BarrierOnly,
}

impl PricingModel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WarrantIntrinsic => "warrant_intrinsic",
            Self::FinancingLevel => "financing_level",
            Self::BarrierOnly => "barrier_only",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PricingSpec {
    pub family: ProductFamily,
    pub model: PricingModel,
    pub payoff_reference: f64,
    pub reference_currency: String,
    pub barrier: Option<f64>,
    pub barrier_currency: Option<String>,
}

pub fn pricing_spec(product: &StructuredProduct) -> Option<PricingSpec> {
    let family = classify_product(product);
    let detail = product.detail.as_ref();
    let reference_currency = detail
        .and_then(|detail| detail.strike_currency.clone())
        .unwrap_or_default();
    let barrier = detail
        .and_then(|detail| detail.lower_threshold)
        .or(product.strike);
    let barrier_currency = detail.and_then(|detail| detail.strike_currency.clone());

    match family {
        ProductFamily::Warrant => Some(PricingSpec {
            family,
            model: PricingModel::WarrantIntrinsic,
            payoff_reference: detail
                .and_then(|detail| detail.strike_price)
                .or(product.strike)?,
            reference_currency,
            barrier: None,
            barrier_currency: None,
        }),
        ProductFamily::MiniFuture | ProductFamily::Turbo => Some(PricingSpec {
            family,
            model: PricingModel::FinancingLevel,
            payoff_reference: detail
                .and_then(|detail| detail.second_strike_price)
                .or_else(|| detail.and_then(|detail| detail.strike_price))
                .or(product.strike)?,
            reference_currency,
            barrier,
            barrier_currency,
        }),
        ProductFamily::OpenEndKnockOut => {
            let model = if detail.and_then(|detail| detail.second_strike_price).is_some() {
                PricingModel::FinancingLevel
            } else {
                PricingModel::BarrierOnly
            };
            Some(PricingSpec {
                family,
                model,
                payoff_reference: detail
                    .and_then(|detail| detail.second_strike_price)
                    .or_else(|| detail.and_then(|detail| detail.strike_price))
                    .or(product.strike)?,
                reference_currency,
                barrier,
                barrier_currency,
            })
        }
        ProductFamily::Certificate | ProductFamily::Other => None,
    }
}

pub fn classify_product(product: &StructuredProduct) -> ProductFamily {
    let text = [
        product.product_type.as_str(),
        product.name.as_deref().unwrap_or_default(),
        product
            .detail
            .as_ref()
            .and_then(|detail| detail.marketing_product_name.as_deref())
            .unwrap_or_default(),
    ]
    .join(" ")
    .to_ascii_lowercase();

    if text.contains("mini-future") || text.contains("mini future") {
        ProductFamily::MiniFuture
    } else if text.contains("open-end knock-out") || text.contains("knock-out") {
        ProductFamily::OpenEndKnockOut
    } else if text.contains("turbo") {
        ProductFamily::Turbo
    } else if text.contains("certificate") || text.contains("certificat") {
        ProductFamily::Certificate
    } else if contains_word(&text, "warrant") {
        ProductFamily::Warrant
    } else {
        ProductFamily::Other
    }
}

fn contains_word(text: &str, needle: &str) -> bool {
    text.split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(|word| word == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::structured::{
        Direction, ListedPrice, Maturity, Moneyness, ProductDetail, StructuredProduct,
    };

    #[test]
    fn classifies_warrant_and_uses_strike_as_payoff_reference() {
        let product = product("Warrant Call", Some(280.0), None, None);
        let spec = pricing_spec(&product).unwrap();

        assert_eq!(spec.family, ProductFamily::Warrant);
        assert_eq!(spec.model, PricingModel::WarrantIntrinsic);
        assert_eq!(spec.payoff_reference, 280.0);
        assert_eq!(spec.barrier, None);
    }

    #[test]
    fn classifies_mini_future_and_prefers_financing_level() {
        let product = product("Mini-Future Short", Some(303.93), Some(323.3388), Some(303.93));
        let spec = pricing_spec(&product).unwrap();

        assert_eq!(spec.family, ProductFamily::MiniFuture);
        assert_eq!(spec.model, PricingModel::FinancingLevel);
        assert_eq!(spec.payoff_reference, 323.3388);
        assert_eq!(spec.barrier, Some(303.93));
    }

    #[test]
    fn open_end_knock_out_without_second_strike_falls_back_to_barrier_only() {
        let product = product(
            "Open-End Knock-Out Warrant Put",
            Some(352.86),
            None,
            Some(352.86),
        );
        let spec = pricing_spec(&product).unwrap();

        assert_eq!(spec.family, ProductFamily::OpenEndKnockOut);
        assert_eq!(spec.model, PricingModel::BarrierOnly);
        assert_eq!(spec.payoff_reference, 352.86);
    }

    fn product(
        product_type: &str,
        strike: Option<f64>,
        second_strike: Option<f64>,
        lower_threshold: Option<f64>,
    ) -> StructuredProduct {
        StructuredProduct {
            symbol: "TEST".to_string(),
            boursorama_symbol: "1rPTEST".to_string(),
            boursorama_url: "https://example.test".to_string(),
            yahoo_symbol: None,
            isin: None,
            mic: None,
            name: Some(product_type.to_string()),
            underlying: "APPLE".to_string(),
            product_type: product_type.to_string(),
            direction: Direction::Call,
            strike,
            maturity: Maturity::OpenEnd,
            bid_ask: "/".to_string(),
            last_price: ListedPrice {
                currency: Some("EUR".to_string()),
                value: Some(1.0),
            },
            last_trade_time: String::new(),
            distance_to_spot_pct: None,
            moneyness: Moneyness::InTheMoney,
            detail_path: None,
            detail: Some(detail(product_type, strike, second_strike, lower_threshold)),
            boursorama_quote: None,
        }
    }

    fn detail(
        product_type: &str,
        strike: Option<f64>,
        second_strike: Option<f64>,
        lower_threshold: Option<f64>,
    ) -> ProductDetail {
        ProductDetail {
            short_name: None,
            long_name: None,
            issuer_name: None,
            quote_currency: Some("EUR".to_string()),
            issue_date: None,
            issue_price: None,
            issue_price_currency: None,
            introduction_date: None,
            parity_warrant_underlying: None,
            parity_underlying_warrant: None,
            parity_first_warrant_underlying: None,
            warrants_per_underlying: Some(10.0),
            strike_price: strike,
            strike_currency: Some("USD".to_string()),
            second_strike_price: second_strike,
            second_strike_currency: Some("USD".to_string()),
            leverage_level: None,
            lower_threshold,
            marketing_product_name: Some(product_type.to_string()),
            underlying_designation: Some("APPLE".to_string()),
            underlying_group_name: None,
            underlying_isin: None,
            kid_url: None,
            opening_time: None,
            closing_time: None,
            trading_open_time: None,
            trading_close_time: None,
            trading_lot: None,
            number_of_shares: None,
            price_multiplier: None,
            trading_status: Some("CLO".to_string()),
            quotation_state: Some("AUT".to_string()),
            last_price: Some(1.0),
            last_quantity: None,
            open_price: None,
            close_price: None,
            previous_close_price: None,
            high_price: None,
            low_price: None,
            valorization: None,
            valorization_datetime: None,
            traded_quantity: None,
            traded_amount: None,
            trade_count: None,
            last_update: None,
            last_quote_datetime: None,
            last_trade_type: None,
            halt_reason: None,
            quality: None,
            listed: Some(true),
        }
    }
}
