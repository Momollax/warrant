use chrono::NaiveDate;

use crate::models::euronext::{EuronextProduct, InstrumentDetail};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
pub enum Direction {
    Call,
    Put,
    Unknown,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Moneyness {
    InTheMoney,
    AtTheMoney,
    OutOfTheMoney,
    Unknown,
}

impl Moneyness {
    pub fn as_str(self) -> &'static str {
        match self {
            Moneyness::InTheMoney => "itm",
            Moneyness::AtTheMoney => "atm",
            Moneyness::OutOfTheMoney => "otm",
            Moneyness::Unknown => "unknown",
        }
    }
}

impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::Call => "call",
            Direction::Put => "put",
            Direction::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Maturity {
    Date(NaiveDate),
    OpenEnd,
    Unknown,
}

impl Maturity {
    pub fn sort_key(&self) -> (u8, Option<NaiveDate>) {
        match self {
            Maturity::Date(date) => (0, Some(*date)),
            Maturity::OpenEnd => (1, None),
            Maturity::Unknown => (2, None),
        }
    }

    pub fn as_string(&self) -> String {
        match self {
            Maturity::Date(date) => date.to_string(),
            Maturity::OpenEnd => "open-end".to_string(),
            Maturity::Unknown => String::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ListedPrice {
    pub currency: Option<String>,
    pub value: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct StructuredProduct {
    pub symbol: String,
    pub boursorama_symbol: String,
    pub boursorama_url: String,
    pub yahoo_symbol: Option<String>,
    pub isin: Option<String>,
    pub mic: Option<String>,
    pub name: Option<String>,
    pub underlying: String,
    pub product_type: String,
    pub direction: Direction,
    pub strike: Option<f64>,
    pub maturity: Maturity,
    pub bid_ask: String,
    pub last_price: ListedPrice,
    pub last_trade_time: String,
    pub distance_to_spot_pct: Option<f64>,
    pub moneyness: Moneyness,
    pub detail_path: Option<String>,
    pub detail: Option<ProductDetail>,
    pub boursorama_quote: Option<BoursoramaQuote>,
}

#[derive(Debug, Clone)]
pub struct BoursoramaQuote {
    pub symbol: String,
    pub url: String,
    pub currency: Option<String>,
    pub last: Option<f64>,
    pub previous_close: Option<f64>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub total_volume: Option<f64>,
    pub variation: Option<f64>,
    pub trade_date: Option<String>,
    pub bid: Option<f64>,
    pub ask: Option<f64>,
    pub bid_size: Option<f64>,
    pub ask_size: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct ProductDetail {
    pub short_name: Option<String>,
    pub long_name: Option<String>,
    pub issuer_name: Option<String>,
    pub quote_currency: Option<String>,
    pub issue_date: Option<String>,
    pub issue_price: Option<f64>,
    pub issue_price_currency: Option<String>,
    pub introduction_date: Option<String>,
    pub parity_warrant_underlying: Option<f64>,
    pub parity_underlying_warrant: Option<f64>,
    pub parity_first_warrant_underlying: Option<f64>,
    pub warrants_per_underlying: Option<f64>,
    pub strike_price: Option<f64>,
    pub strike_currency: Option<String>,
    pub second_strike_price: Option<f64>,
    pub second_strike_currency: Option<String>,
    pub leverage_level: Option<f64>,
    pub lower_threshold: Option<f64>,
    pub marketing_product_name: Option<String>,
    pub underlying_designation: Option<String>,
    pub underlying_group_name: Option<String>,
    pub underlying_isin: Option<String>,
    pub kid_url: Option<String>,
    pub opening_time: Option<String>,
    pub closing_time: Option<String>,
    pub trading_open_time: Option<String>,
    pub trading_close_time: Option<String>,
    pub trading_lot: Option<f64>,
    pub number_of_shares: Option<f64>,
    pub price_multiplier: Option<f64>,
    pub trading_status: Option<String>,
    pub quotation_state: Option<String>,
    pub last_price: Option<f64>,
    pub last_quantity: Option<f64>,
    pub open_price: Option<f64>,
    pub close_price: Option<f64>,
    pub previous_close_price: Option<f64>,
    pub high_price: Option<f64>,
    pub low_price: Option<f64>,
    pub valorization: Option<f64>,
    pub valorization_datetime: Option<String>,
    pub traded_quantity: Option<f64>,
    pub traded_amount: Option<f64>,
    pub trade_count: Option<u64>,
    pub last_update: Option<String>,
    pub last_quote_datetime: Option<String>,
    pub last_trade_type: Option<String>,
    pub halt_reason: Option<String>,
    pub quality: Option<String>,
    pub listed: Option<bool>,
}

impl StructuredProduct {
    pub fn from_euronext(product: EuronextProduct, spot_price: f64) -> Self {
        let direction = infer_direction(&product);
        let strike = parse_decimal(&product.strike);
        let maturity = parse_maturity(&product.maturity);
        let last_price = parse_listed_price(&product.last_price);
        let distance_to_spot_pct = strike.map(|value| (value - spot_price) / spot_price * 100.0);
        let moneyness = classify_moneyness(direction, strike, spot_price);
        let (boursorama_symbol, boursorama_url) =
            boursorama_identity(&product.symbol, product.detail_path.as_deref());

        Self {
            boursorama_symbol,
            boursorama_url,
            symbol: product.symbol,
            yahoo_symbol: product.yahoo_symbol,
            isin: product.isin,
            mic: product.mic,
            name: product.name,
            underlying: product.underlying,
            product_type: product.product_type,
            direction,
            strike,
            maturity,
            bid_ask: product.bid_ask,
            last_price,
            last_trade_time: product.last_trade_time,
            distance_to_spot_pct,
            moneyness,
            detail_path: product.detail_path,
            detail: None,
            boursorama_quote: None,
        }
    }

    pub fn with_detail(mut self, detail: InstrumentDetail) -> Self {
        self.detail = Some(ProductDetail::from_instrument_detail(detail));
        self
    }

    pub fn with_boursorama_quote(mut self, quote: BoursoramaQuote) -> Self {
        self.boursorama_quote = Some(quote);
        self
    }

    pub fn set_boursorama_quote(&mut self, quote: BoursoramaQuote) {
        self.boursorama_quote = Some(quote);
    }
}

fn boursorama_identity(symbol: &str, detail_path: Option<&str>) -> (String, String) {
    if let Some(path) = detail_path {
        if path.contains("/bourse/produits-de-bourse/cours/") {
            let boursorama_symbol = path
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .filter(|value| !value.is_empty())
                .unwrap_or(symbol)
                .to_string();
            let url = if path.starts_with("http") {
                path.to_string()
            } else {
                format!("https://www.boursorama.com{path}")
            };
            return (boursorama_symbol, url);
        }
    }

    (
        format!("1rP{symbol}"),
        format!("https://www.boursorama.com/bourse/produits-de-bourse/cours/1rP{symbol}"),
    )
}

impl ProductDetail {
    fn from_instrument_detail(detail: InstrumentDetail) -> Self {
        let current_session = detail.current_session;
        let spec_features = detail.spec_features.unwrap_or_default();
        let daily_perf = detail.perf.as_deref().and_then(find_daily_performance);
        let trading_group = detail.trading_group;
        let opening_time = find_spec_string(&spec_features, "openingTime");
        let closing_time = find_spec_string(&spec_features, "closingTime");
        let parity_warrant_underlying = detail.parity_warrant_underlying.as_deref().and_then(parse_decimal);
        let parity_underlying_warrant = detail.parity_underlying_warrant.as_deref().and_then(parse_decimal);
        let parity_first_warrant_underlying = find_spec_decimal(
            &spec_features,
            "parityFirstWarrantUnderlying",
        );

        Self {
            short_name: detail.short_name,
            long_name: detail.long_name,
            issuer_name: detail.issuer_name,
            quote_currency: detail.currency,
            issue_date: detail.issue_date.as_deref().map(format_euronext_date),
            issue_price: detail.issue_price.as_deref().and_then(parse_decimal),
            issue_price_currency: detail.issue_price_currency,
            introduction_date: detail.introduction_date.as_deref().map(format_euronext_date),
            parity_warrant_underlying,
            parity_underlying_warrant,
            parity_first_warrant_underlying,
            warrants_per_underlying: warrants_per_underlying_from_parity(
                parity_warrant_underlying,
                parity_underlying_warrant,
                parity_first_warrant_underlying,
            ),
            strike_price: detail.strike_px.as_deref().and_then(parse_decimal),
            strike_currency: detail.strike_currency,
            second_strike_price: detail.second_strike_px.as_deref().and_then(parse_decimal),
            second_strike_currency: detail.second_strike_currency,
            leverage_level: find_spec_decimal(&spec_features, "leverageLevel"),
            lower_threshold: find_spec_decimal(&spec_features, "lowerThreshold"),
            marketing_product_name: find_spec_string(&spec_features, "marketingProductName"),
            underlying_designation: find_spec_string(&spec_features, "underlyingDesignation"),
            underlying_group_name: find_spec_string(&spec_features, "underlyingGroupName"),
            underlying_isin: find_spec_string(&spec_features, "underlyingIsinCode"),
            kid_url: find_spec_string(&spec_features, "kid_BEL_FR")
                .or_else(|| find_spec_string(&spec_features, "kid_FRA_FR")),
            opening_time,
            closing_time,
            trading_open_time: trading_group
                .as_ref()
                .and_then(|group| group.time_opening_1.as_deref())
                .map(format_euronext_datetime),
            trading_close_time: trading_group
                .as_ref()
                .and_then(|group| group.time_closing_1.as_deref())
                .map(format_euronext_datetime),
            trading_lot: detail.trading_lot.as_deref().and_then(parse_decimal),
            number_of_shares: detail.number_of_shares.as_deref().and_then(parse_decimal),
            price_multiplier: detail
                .price_multiplier
                .as_deref()
                .and_then(parse_decimal)
                .or_else(|| find_spec_decimal(&spec_features, "priceMultiplier")),
            trading_status: current_session
                .as_ref()
                .and_then(|session| session.trading_status.clone()),
            quotation_state: current_session
                .as_ref()
                .and_then(|session| session.quotation_state.clone()),
            last_price: current_session
                .as_ref()
                .and_then(|session| session.last_price.as_deref())
                .and_then(parse_decimal),
            last_quantity: current_session
                .as_ref()
                .and_then(|session| session.last_quantity.as_deref())
                .and_then(parse_decimal),
            open_price: current_session
                .as_ref()
                .and_then(|session| session.open_price.as_deref())
                .and_then(parse_decimal),
            close_price: current_session
                .as_ref()
                .and_then(|session| session.close_price.as_deref())
                .and_then(parse_decimal),
            previous_close_price: current_session
                .as_ref()
                .and_then(|session| session.previous_adjusted_close_price.as_deref())
                .and_then(parse_decimal)
                .or_else(|| find_spec_decimal(&spec_features, "closingPrice")),
            high_price: daily_perf
                .and_then(|performance| performance.high_price.as_deref())
                .and_then(parse_decimal),
            low_price: daily_perf
                .and_then(|performance| performance.low_price.as_deref())
                .and_then(parse_decimal),
            valorization: current_session
                .as_ref()
                .and_then(|session| session.valorization.as_deref())
                .and_then(parse_decimal),
            valorization_datetime: current_session
                .as_ref()
                .and_then(|session| session.valorization_datetime.as_deref())
                .map(format_euronext_datetime),
            traded_quantity: current_session
                .as_ref()
                .and_then(|session| session.traded_quantity.as_deref())
                .and_then(parse_decimal),
            traded_amount: daily_perf
                .and_then(|performance| performance.traded_amount.as_deref())
                .and_then(parse_decimal),
            trade_count: current_session
                .as_ref()
                .and_then(|session| session.trade_count.as_deref())
                .and_then(|value| value.parse().ok()),
            last_update: current_session
                .as_ref()
                .and_then(|session| session.last_update.as_deref())
                .map(format_euronext_datetime),
            last_quote_datetime: daily_perf
                .and_then(|performance| performance.last_quote_datetime.as_deref())
                .map(format_euronext_datetime),
            last_trade_type: current_session
                .as_ref()
                .and_then(|session| session.last_trade_type.clone()),
            halt_reason: current_session
                .as_ref()
                .and_then(|session| session.halt_reason.clone()),
            quality: detail.quality,
            listed: detail.listed.as_deref().and_then(parse_bool),
        }
    }
}

fn warrants_per_underlying_from_parity(
    parity_warrant_underlying: Option<f64>,
    parity_underlying_warrant: Option<f64>,
    parity_first_warrant_underlying: Option<f64>,
) -> Option<f64> {
    [
        parity_warrant_underlying,
        parity_underlying_warrant,
        parity_first_warrant_underlying,
    ]
        .into_iter()
        .flatten()
        .filter(|value| value.is_finite() && *value > 0.0)
        .map(|value| if value >= 1.0 { value } else { 1.0 / value })
        .max_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal))
}

fn find_spec_decimal(
    spec_features: &[crate::models::euronext::InstrumentSpecFeature],
    field: &str,
) -> Option<f64> {
    spec_features
        .iter()
        .find(|feature| feature.field == field)
        .and_then(|feature| feature.value.as_deref())
        .and_then(parse_decimal)
}

fn find_spec_string(
    spec_features: &[crate::models::euronext::InstrumentSpecFeature],
    field: &str,
) -> Option<String> {
    spec_features
        .iter()
        .find(|feature| feature.field == field)
        .and_then(|feature| feature.value.clone())
}

fn find_daily_performance(
    performances: &[crate::models::euronext::InstrumentPerformance],
) -> Option<&crate::models::euronext::InstrumentPerformance> {
    performances
        .iter()
        .find(|performance| performance.period_type.as_deref() == Some("D"))
}

fn parse_bool(value: &str) -> Option<bool> {
    match value {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

fn format_euronext_date(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() == 8 && trimmed.chars().all(|ch| ch.is_ascii_digit()) {
        format!("{}-{}-{}", &trimmed[0..4], &trimmed[4..6], &trimmed[6..8])
    } else {
        trimmed.to_string()
    }
}

fn format_euronext_datetime(value: &str) -> String {
    let trimmed = value.trim();
    if let Some((date, time)) = trimmed.split_once('-') {
        if date.len() == 8 && time.contains(':') {
            return format!(
                "{}-{}-{} {}",
                &date[0..4],
                &date[4..6],
                &date[6..8],
                time
            );
        }
        if date.len() == 8 && time.len() >= 6 {
            return format!(
                "{}-{}-{} {}:{}:{}",
                &date[0..4],
                &date[4..6],
                &date[6..8],
                &time[0..2],
                &time[2..4],
                &time[4..6]
            );
        }
    }
    trimmed.to_string()
}

fn classify_moneyness(direction: Direction, strike: Option<f64>, spot_price: f64) -> Moneyness {
    let Some(strike) = strike else {
        return Moneyness::Unknown;
    };
    if spot_price <= 0.0 {
        return Moneyness::Unknown;
    }

    let distance_abs_pct = ((strike - spot_price) / spot_price * 100.0).abs();
    if distance_abs_pct <= 1.0 {
        return Moneyness::AtTheMoney;
    }

    match direction {
        Direction::Call if spot_price > strike => Moneyness::InTheMoney,
        Direction::Call => Moneyness::OutOfTheMoney,
        Direction::Put if spot_price < strike => Moneyness::InTheMoney,
        Direction::Put => Moneyness::OutOfTheMoney,
        Direction::Unknown => Moneyness::Unknown,
    }
}

fn infer_direction(product: &EuronextProduct) -> Direction {
    let text = format!(
        "{} {}",
        product.product_type.to_lowercase(),
        product.name.as_deref().unwrap_or_default().to_lowercase()
    );

    if contains_word(&text, "call") || contains_word(&text, "long") {
        Direction::Call
    } else if contains_word(&text, "put") || contains_word(&text, "short") {
        Direction::Put
    } else {
        Direction::Unknown
    }
}

fn contains_word(text: &str, needle: &str) -> bool {
    text.split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(|word| word == needle)
}

fn parse_maturity(value: &str) -> Maturity {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "-" {
        return Maturity::OpenEnd;
    }

    NaiveDate::parse_from_str(trimmed, "%Y-%m-%d")
        .map(Maturity::Date)
        .unwrap_or(Maturity::Unknown)
}

fn parse_listed_price(value: &str) -> ListedPrice {
    let mut parts = value.split_whitespace();
    let first = parts.next();
    let second = parts.next();

    match (first, second) {
        (Some(currency), Some(price)) if currency.chars().all(|ch| ch.is_ascii_uppercase()) => {
            ListedPrice {
                currency: Some(currency.to_string()),
                value: parse_decimal(price),
            }
        }
        (Some(price), _) => ListedPrice {
            currency: None,
            value: parse_decimal(price),
        },
        _ => ListedPrice {
            currency: None,
            value: None,
        },
    }
}

fn parse_decimal(value: &str) -> Option<f64> {
    let normalized = value.trim().replace(' ', "").replace(',', ".");
    if normalized.is_empty() || normalized == "-" || normalized == "/" {
        return None;
    }
    normalized.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_decimal_with_comma_and_rejects_missing_markers() {
        assert_eq!(parse_decimal("4,440"), Some(4.44));
        assert_eq!(parse_decimal(" 250.00 "), Some(250.0));
        assert_eq!(parse_decimal("-"), None);
        assert_eq!(parse_decimal("/"), None);
        assert_eq!(parse_decimal(""), None);
    }

    #[test]
    fn parses_listed_price_with_or_without_currency() {
        let eur_price = parse_listed_price("EUR 4,4400");
        assert_eq!(eur_price.currency.as_deref(), Some("EUR"));
        assert_eq!(eur_price.value, Some(4.44));

        let bare_price = parse_listed_price("3.9850");
        assert_eq!(bare_price.currency, None);
        assert_eq!(bare_price.value, Some(3.985));
    }

    #[test]
    fn parses_open_end_and_dated_maturities() {
        assert_eq!(parse_maturity("-"), Maturity::OpenEnd);
        assert_eq!(parse_maturity(""), Maturity::OpenEnd);
        assert_eq!(
            parse_maturity("2026-06-18"),
            Maturity::Date(NaiveDate::from_ymd_opt(2026, 6, 18).unwrap())
        );
        assert_eq!(parse_maturity("18/06/26"), Maturity::Unknown);
    }

    #[test]
    fn normalizes_parity_to_warrants_per_underlying() {
        assert_eq!(
            warrants_per_underlying_from_parity(Some(10.0), None, None),
            Some(10.0)
        );
        assert_eq!(
            warrants_per_underlying_from_parity(None, None, Some(0.02)),
            Some(50.0)
        );
        assert_eq!(
            warrants_per_underlying_from_parity(Some(1.0), Some(10.0), Some(0.1)),
            Some(10.0)
        );
        assert_eq!(warrants_per_underlying_from_parity(None, None, None), None);
    }
}
