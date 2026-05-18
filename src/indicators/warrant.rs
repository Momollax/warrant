use std::cmp::Ordering;
use std::collections::BTreeMap;

use chrono::{NaiveDate, Utc};

use crate::indicators::options::{implied_volatility, median_implied_volatility, OptionKind};
use crate::models::fx::FxRateBook;
use crate::models::structured::{Direction, Maturity, StructuredProduct};
use crate::pricing::{pricing_spec, PricingModel};

const MIN_PEERS: usize = 3;
const MAX_BOURSORAMA_RELATIVE_SPREAD: f64 = 0.25;
const MAX_BOURSORAMA_REFERENCE_DEVIATION: f64 = 0.50;
const MIN_ABSOLUTE_PRICE_TOLERANCE: f64 = 0.05;
const MAX_EXECUTABLE_RELATIVE_SPREAD: f64 = 0.25;
const MIN_FINANCING_PRICE_TO_INTRINSIC: f64 = 0.50;
const MAX_FINANCING_PRICE_TO_INTRINSIC: f64 = 3.00;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ValuationSide {
    Undervalued,
    Overvalued,
}

impl ValuationSide {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Undervalued => "undervalued",
            Self::Overvalued => "overvalued",
        }
    }
}

#[derive(Debug, Clone)]
pub struct OpportunitySignal {
    pub symbol: String,
    pub web_url: String,
    pub side: String,
    pub moneyness: String,
    pub product_family: String,
    pub pricing_model: String,
    pub product_type: String,
    pub maturity: String,
    pub price_currency: String,
    pub last_price: f64,
    pub price_source: String,
    pub bid_price: Option<f64>,
    pub ask_price: Option<f64>,
    pub mid_price: Option<f64>,
    pub spread_pct: Option<f64>,
    pub bid_size: Option<f64>,
    pub ask_size: Option<f64>,
    pub quote_volume: Option<f64>,
    pub execution_status: String,
    pub data_quality_score: f64,
    pub liquidity_score: f64,
    pub strike: f64,
    pub strike_currency: String,
    pub barrier: Option<f64>,
    pub barrier_currency: Option<String>,
    pub barrier_distance_pct: Option<f64>,
    pub raw_intrinsic: f64,
    pub intrinsic_per_product: f64,
    pub intrinsic_currency: String,
    pub fx_rate: f64,
    pub fx_source_ticker: String,
    pub warrants_per_underlying: Option<f64>,
    pub price_to_intrinsic: f64,
    pub effective_gearing: Option<f64>,
    pub premium_discount_pct: f64,
    pub years_to_maturity: Option<f64>,
    pub risk_free_rate: Option<f64>,
    pub dividend_yield: Option<f64>,
    pub implied_volatility: Option<f64>,
    pub smile_median_iv: Option<f64>,
    pub smile_gap_vol_points: Option<f64>,
    pub volatility_signal: String,
    pub metric_kind: String,
    pub relative_metric: f64,
    pub peer_median_relative_metric: f64,
    pub peer_gap_pct: f64,
    pub spread_adjusted_gap_pct: f64,
    pub peer_count: usize,
    pub valuation: ValuationSide,
    pub score: f64,
    pub note: String,
}

pub fn rank_relative_value(
    products: &[StructuredProduct],
    spot_price: f64,
    spot_currency: &str,
    fx_rates: &FxRateBook,
) -> Vec<OpportunitySignal> {
    let mut grouped: BTreeMap<String, Vec<ComparableProduct<'_>>> = BTreeMap::new();

    for product in products {
        if let Some(comparable) =
            ComparableProduct::from_product(product, spot_price, spot_currency, fx_rates)
        {
            grouped
                .entry(comparable.bucket_key())
                .or_default()
                .push(comparable);
        }
    }

    let mut signals = Vec::new();
    for peers in grouped.values() {
        if peers.len() < MIN_PEERS {
            continue;
        }

        let median = median(
            peers
                .iter()
                .map(|peer| peer.relative_metric)
                .collect::<Vec<_>>(),
        );

        if median <= 0.0 {
            continue;
        }
        let smile_median_iv =
            median_implied_volatility(peers.iter().map(|peer| peer.implied_volatility));

        for peer in peers {
            let peer_gap_pct = (median - peer.relative_metric) / median * 100.0;
            if peer_gap_pct.abs() <= f64::EPSILON {
                continue;
            }

            signals.push(peer.to_signal(median, peer_gap_pct, peers.len(), smile_median_iv));
        }
    }

    signals.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.symbol.cmp(&right.symbol))
    });
    signals
}

pub fn standalone_product_signals(
    products: &[StructuredProduct],
    spot_price: f64,
    spot_currency: &str,
    fx_rates: &FxRateBook,
) -> Vec<OpportunitySignal> {
    let mut grouped: BTreeMap<String, Vec<ComparableProduct<'_>>> = BTreeMap::new();

    for product in products {
        if let Some(comparable) =
            ComparableProduct::from_product(product, spot_price, spot_currency, fx_rates)
        {
            grouped
                .entry(comparable.bucket_key())
                .or_default()
                .push(comparable);
        }
    }

    let mut signals = Vec::new();
    for peers in grouped.values() {
        let median = median(
            peers
                .iter()
                .map(|peer| peer.relative_metric)
                .collect::<Vec<_>>(),
        );
        let smile_median_iv =
            median_implied_volatility(peers.iter().map(|peer| peer.implied_volatility));

        for peer in peers {
            let peer_gap_pct = if median > 0.0 {
                (median - peer.relative_metric) / median * 100.0
            } else {
                0.0
            };
            signals.push(peer.to_signal(median, peer_gap_pct, peers.len(), smile_median_iv));
        }
    }

    signals.sort_by(|left, right| {
        left.product_family
            .cmp(&right.product_family)
            .then_with(|| left.side.cmp(&right.side))
            .then_with(|| left.maturity.cmp(&right.maturity))
            .then_with(|| {
                left.strike
                    .partial_cmp(&right.strike)
                    .unwrap_or(Ordering::Equal)
            })
            .then_with(|| left.symbol.cmp(&right.symbol))
    });
    signals
}

pub fn scenario_warrant_signals(
    products: &[StructuredProduct],
    spot_price: f64,
    spot_currency: &str,
    fx_rates: &FxRateBook,
) -> Vec<OpportunitySignal> {
    let mut grouped: BTreeMap<String, Vec<ComparableProduct<'_>>> = BTreeMap::new();

    for product in products {
        if let Some(comparable) =
            ComparableProduct::from_warrant_for_scenario(product, spot_price, spot_currency, fx_rates)
        {
            grouped
                .entry(comparable.bucket_key())
                .or_default()
                .push(comparable);
        }
    }

    let mut signals = Vec::new();
    for peers in grouped.values() {
        let median = median(
            peers
                .iter()
                .map(|peer| peer.relative_metric)
                .collect::<Vec<_>>(),
        );
        let smile_median_iv =
            median_implied_volatility(peers.iter().map(|peer| peer.implied_volatility));

        for peer in peers {
            let peer_gap_pct = if median > 0.0 {
                (median - peer.relative_metric) / median * 100.0
            } else {
                0.0
            };
            signals.push(peer.to_signal(median, peer_gap_pct, peers.len(), smile_median_iv));
        }
    }

    signals.sort_by(|left, right| {
        left.side
            .cmp(&right.side)
            .then_with(|| left.maturity.cmp(&right.maturity))
            .then_with(|| {
                left.strike
                    .partial_cmp(&right.strike)
                    .unwrap_or(Ordering::Equal)
            })
            .then_with(|| left.symbol.cmp(&right.symbol))
    });
    signals
}

#[derive(Clone)]
struct ComparableProduct<'a> {
    product: &'a StructuredProduct,
    product_family: &'static str,
    pricing_model: &'static str,
    strike: f64,
    strike_currency: String,
    barrier: Option<f64>,
    barrier_currency: Option<String>,
    barrier_distance_pct: Option<f64>,
    reference_distance_to_spot_pct: f64,
    last_price: f64,
    price_source: &'static str,
    bid_price: Option<f64>,
    ask_price: Option<f64>,
    mid_price: Option<f64>,
    spread_pct: Option<f64>,
    bid_size: Option<f64>,
    ask_size: Option<f64>,
    quote_volume: Option<f64>,
    execution_status: &'static str,
    data_quality_score: f64,
    liquidity_score: f64,
    raw_intrinsic: f64,
    intrinsic_per_product: f64,
    intrinsic_currency: String,
    fx_rate: f64,
    fx_source_ticker: String,
    warrants_per_underlying: Option<f64>,
    price_to_intrinsic: f64,
    effective_gearing: Option<f64>,
    premium_discount_pct: f64,
    years_to_maturity: Option<f64>,
    risk_free_rate: Option<f64>,
    dividend_yield: Option<f64>,
    implied_volatility: Option<f64>,
    metric_kind: &'static str,
    relative_metric: f64,
}

impl<'a> ComparableProduct<'a> {
    fn from_product(
        product: &'a StructuredProduct,
        spot_price: f64,
        spot_currency: &'a str,
        fx_rates: &FxRateBook,
    ) -> Option<Self> {
        Self::from_product_inner(product, spot_price, spot_currency, fx_rates, true)
    }

    fn from_warrant_for_scenario(
        product: &'a StructuredProduct,
        spot_price: f64,
        spot_currency: &'a str,
        fx_rates: &FxRateBook,
    ) -> Option<Self> {
        Self::from_product_inner(product, spot_price, spot_currency, fx_rates, false)
    }

    fn from_product_inner(
        product: &'a StructuredProduct,
        spot_price: f64,
        spot_currency: &'a str,
        fx_rates: &FxRateBook,
        require_positive_intrinsic: bool,
    ) -> Option<Self> {
        if !is_exploitable(product) {
            return None;
        }

        if !matches!(product.direction, Direction::Call | Direction::Put) {
            return None;
        }
        let spec = pricing_spec(product)?;
        let pricing_model = spec.model;
        if !require_positive_intrinsic && pricing_model != PricingModel::WarrantIntrinsic {
            return None;
        }
        let reference_currency = if spec.reference_currency.is_empty() {
            spot_currency.to_string()
        } else {
            spec.reference_currency.clone()
        };
        let quote_price = buy_quote_price(product)?;
        let price_currency = quote_price
            .currency
            .as_deref()
            .or(product.last_price.currency.as_deref())
            .or_else(|| {
                product
                    .detail
                    .as_ref()
                    .and_then(|detail| detail.quote_currency.as_deref())
            })?
            .to_string();
        let spot_in_reference_currency =
            fx_rates.convert(spot_price, spot_currency, &reference_currency)?;
        let strike = spec.payoff_reference;
        let last_price = quote_price.value;
        let raw_intrinsic = raw_intrinsic(product.direction, spot_in_reference_currency, strike);
        if last_price <= 0.0 || (require_positive_intrinsic && raw_intrinsic <= 0.0) {
            return None;
        }

        let warrants_per_underlying = warrants_per_underlying(product);
        let intrinsic_per_product_in_strike_currency = match warrants_per_underlying {
            Some(parity) if parity > 0.0 && raw_intrinsic > 0.0 => raw_intrinsic / parity,
            _ if raw_intrinsic > 0.0 => raw_intrinsic,
            _ => 0.0,
        };
        let fx_rate = fx_rates.rate(&reference_currency, &price_currency)?;
        let fx_source_ticker = fx_rates
            .source_ticker(&reference_currency, &price_currency)
            .unwrap_or("UNKNOWN")
            .to_string();
        let intrinsic_per_product =
            intrinsic_per_product_in_strike_currency * fx_rate;

        if require_positive_intrinsic && intrinsic_per_product <= 0.0 {
            return None;
        }

        let price_to_intrinsic = if intrinsic_per_product > 0.0 {
            last_price / intrinsic_per_product
        } else {
            0.0
        };
        let (metric_kind, relative_metric) = relative_metric_for_product(
            product.direction,
            pricing_model,
            spot_in_reference_currency,
            strike,
            last_price,
            &price_currency,
            &reference_currency,
            warrants_per_underlying,
            fx_rates,
            price_to_intrinsic,
        )?;
        if !is_relative_metric_plausible(pricing_model, relative_metric) {
            return None;
        }
        let bid_price = quote_price.bid;
        let ask_price = quote_price.ask;
        let mid_price = midpoint(bid_price, ask_price);
        let spread_pct = spread_pct(bid_price, ask_price);
        let execution_status = execution_status(&quote_price);
        let liquidity_score = liquidity_score(&quote_price);
        let data_quality_score = data_quality_score(&quote_price, spread_pct);
        let barrier_distance_pct =
            barrier_distance_pct(product.direction, spot_in_reference_currency, spec.barrier);
        let effective_gearing = effective_gearing(
            spot_in_reference_currency,
            &reference_currency,
            &price_currency,
            last_price,
            warrants_per_underlying,
            fx_rates,
        );
        let premium_discount_pct = match pricing_model {
            PricingModel::WarrantIntrinsic => relative_metric,
            PricingModel::FinancingLevel | PricingModel::BarrierOnly => {
                (price_to_intrinsic - 1.0) * 100.0
            }
        };
        let years_to_maturity = years_to_maturity(&product.maturity, Utc::now().date_naive());
        let risk_free_rate = Some(option_risk_free_rate());
        let dividend_yield = Some(option_dividend_yield());
        let implied_volatility = match (pricing_model, years_to_maturity, warrants_per_underlying) {
            (PricingModel::WarrantIntrinsic, Some(years), Some(parity)) => {
                let option_kind = option_kind(product.direction)?;
                let market_price_ref =
                    fx_rates.convert(last_price, &price_currency, &reference_currency)? * parity;
                implied_volatility(
                    option_kind,
                    market_price_ref,
                    spot_in_reference_currency,
                    strike,
                    years,
                    risk_free_rate?,
                    dividend_yield?,
                )
            }
            _ => None,
        };

        Some(Self {
            product,
            product_family: spec.family.as_str(),
            pricing_model: pricing_model.as_str(),
            strike,
            strike_currency: reference_currency,
            barrier: spec.barrier,
            barrier_currency: spec.barrier_currency,
            barrier_distance_pct,
            reference_distance_to_spot_pct: (strike - spot_in_reference_currency)
                / spot_in_reference_currency
                * 100.0,
            last_price,
            price_source: quote_price.source,
            bid_price,
            ask_price,
            mid_price,
            spread_pct,
            bid_size: quote_price.bid_size,
            ask_size: quote_price.ask_size,
            quote_volume: quote_price.volume,
            execution_status,
            data_quality_score,
            liquidity_score,
            raw_intrinsic,
            intrinsic_per_product,
            intrinsic_currency: price_currency,
            fx_rate,
            fx_source_ticker,
            warrants_per_underlying,
            price_to_intrinsic,
            effective_gearing,
            premium_discount_pct,
            years_to_maturity,
            risk_free_rate,
            dividend_yield,
            implied_volatility,
            metric_kind,
            relative_metric,
        })
    }

    fn bucket_key(&self) -> String {
        let distance_bucket = match self.pricing_model {
            "warrant_intrinsic" => distance_band(Some(self.reference_distance_to_spot_pct), 5.0),
            _ => distance_band(Some(self.reference_distance_to_spot_pct), 20.0),
        };

        format!(
            "{}|{}|{}|{}|{}|{}|{}",
            self.product_family,
            self.pricing_model,
            self.product.direction.as_str(),
            self.product.moneyness.as_str(),
            self.product
                .last_price
                .currency
                .as_deref()
                .unwrap_or("UNKNOWN"),
            self.product.maturity.as_string(),
            distance_bucket
        )
    }

    fn to_signal(
        &self,
        peer_median_relative_metric: f64,
        peer_gap_pct: f64,
        peer_count: usize,
        smile_median_iv: Option<f64>,
    ) -> OpportunitySignal {
        let valuation = if peer_gap_pct >= 0.0 {
            ValuationSide::Undervalued
        } else {
            ValuationSide::Overvalued
        };

        let spread_adjusted_gap_pct = spread_adjusted_gap(peer_gap_pct, self.spread_pct);

        OpportunitySignal {
            symbol: self.product.symbol.clone(),
            web_url: self.product.boursorama_url.clone(),
            side: self.product.direction.as_str().to_string(),
            moneyness: self.product.moneyness.as_str().to_string(),
            product_family: self.product_family.to_string(),
            pricing_model: self.pricing_model.to_string(),
            product_type: self.product.product_type.clone(),
            maturity: self.product.maturity.as_string(),
            price_currency: self.intrinsic_currency.clone(),
            last_price: self.last_price,
            price_source: self.price_source.to_string(),
            bid_price: self.bid_price,
            ask_price: self.ask_price,
            mid_price: self.mid_price,
            spread_pct: self.spread_pct,
            bid_size: self.bid_size,
            ask_size: self.ask_size,
            quote_volume: self.quote_volume,
            execution_status: self.execution_status.to_string(),
            data_quality_score: self.data_quality_score,
            liquidity_score: self.liquidity_score,
            strike: self.strike,
            strike_currency: self.strike_currency.clone(),
            barrier: self.barrier,
            barrier_currency: self.barrier_currency.clone(),
            barrier_distance_pct: self.barrier_distance_pct,
            raw_intrinsic: self.raw_intrinsic,
            intrinsic_per_product: self.intrinsic_per_product,
            intrinsic_currency: self.intrinsic_currency.clone(),
            fx_rate: self.fx_rate,
            fx_source_ticker: self.fx_source_ticker.clone(),
            warrants_per_underlying: self.warrants_per_underlying,
            price_to_intrinsic: self.price_to_intrinsic,
            effective_gearing: self.effective_gearing,
            premium_discount_pct: self.premium_discount_pct,
            years_to_maturity: self.years_to_maturity,
            risk_free_rate: self.risk_free_rate,
            dividend_yield: self.dividend_yield,
            implied_volatility: self.implied_volatility,
            smile_median_iv,
            smile_gap_vol_points: self
                .implied_volatility
                .zip(smile_median_iv)
                .map(|(iv, median_iv)| (median_iv - iv) * 100.0),
            volatility_signal: volatility_signal(self.implied_volatility, smile_median_iv)
                .to_string(),
            metric_kind: self.metric_kind.to_string(),
            relative_metric: self.relative_metric,
            peer_median_relative_metric,
            peer_gap_pct,
            spread_adjusted_gap_pct,
            peer_count,
            valuation,
            score: tradable_score(peer_gap_pct, spread_adjusted_gap_pct),
            note: format!(
                "relative_value_rank; metric={}; spread_adjusted; family_model_parity_fx_adjusted; bid_ask/execution_required",
                self.metric_kind
            ),
        }
    }
}

fn spread_adjusted_gap(peer_gap_pct: f64, spread_pct: Option<f64>) -> f64 {
    let spread_pct = spread_pct.unwrap_or(0.0).max(0.0);
    if peer_gap_pct >= 0.0 {
        peer_gap_pct - spread_pct
    } else {
        peer_gap_pct + spread_pct
    }
}

fn tradable_score(peer_gap_pct: f64, spread_adjusted_gap_pct: f64) -> f64 {
    if peer_gap_pct >= 0.0 && spread_adjusted_gap_pct > 0.0 {
        spread_adjusted_gap_pct
    } else if peer_gap_pct < 0.0 && spread_adjusted_gap_pct < 0.0 {
        spread_adjusted_gap_pct.abs()
    } else {
        0.0
    }
}

fn relative_metric_for_product(
    direction: Direction,
    pricing_model: PricingModel,
    spot_in_reference_currency: f64,
    strike: f64,
    last_price: f64,
    price_currency: &str,
    reference_currency: &str,
    warrants_per_underlying: Option<f64>,
    fx_rates: &FxRateBook,
    price_to_intrinsic: f64,
) -> Option<(&'static str, f64)> {
    match pricing_model {
        PricingModel::WarrantIntrinsic => {
            let premium_pct = warrant_premium_pct(
                direction,
                spot_in_reference_currency,
                strike,
                last_price,
                price_currency,
                reference_currency,
                warrants_per_underlying?,
                fx_rates,
            )?;
            Some(("premium_pct", premium_pct))
        }
        PricingModel::FinancingLevel | PricingModel::BarrierOnly => {
            Some(("price_to_intrinsic", price_to_intrinsic))
        }
    }
}

fn is_relative_metric_plausible(pricing_model: PricingModel, metric: f64) -> bool {
    if !metric.is_finite() {
        return false;
    }

    match pricing_model {
        PricingModel::FinancingLevel | PricingModel::BarrierOnly => {
            (MIN_FINANCING_PRICE_TO_INTRINSIC..=MAX_FINANCING_PRICE_TO_INTRINSIC)
                .contains(&metric)
        }
        PricingModel::WarrantIntrinsic => true,
    }
}

fn warrant_premium_pct(
    direction: Direction,
    spot_in_reference_currency: f64,
    strike: f64,
    last_price: f64,
    price_currency: &str,
    reference_currency: &str,
    warrants_per_underlying: f64,
    fx_rates: &FxRateBook,
) -> Option<f64> {
    if spot_in_reference_currency <= 0.0 || warrants_per_underlying <= 0.0 {
        return None;
    }

    let warrant_price_in_reference =
        fx_rates.convert(last_price, price_currency, reference_currency)?;
    let cost_per_underlying = warrant_price_in_reference * warrants_per_underlying;
    let premium_pct = match direction {
        Direction::Call => {
            (strike + cost_per_underlying - spot_in_reference_currency)
                / spot_in_reference_currency
                * 100.0
        }
        Direction::Put => {
            (spot_in_reference_currency + cost_per_underlying - strike)
                / spot_in_reference_currency
                * 100.0
        }
        Direction::Unknown => return None,
    };

    premium_pct.is_finite().then_some(premium_pct)
}

fn is_exploitable(product: &StructuredProduct) -> bool {
    let Some(detail) = product.detail.as_ref() else {
        return false;
    };

    if detail.listed != Some(true) {
        return false;
    }

    if is_matured_or_expired(&product.maturity, Utc::now().date_naive()) {
        return false;
    }

    if detail.quotation_state.as_deref() == Some("HAL") {
        return false;
    }

    if matches!(detail.trading_status.as_deref(), Some("HAL" | "SUS")) {
        return false;
    }

    let price = detail.last_price.or(product.last_price.value).unwrap_or(0.0);
    if price <= 0.0 {
        return false;
    }

    true
}

fn is_matured_or_expired(maturity: &Maturity, today: NaiveDate) -> bool {
    matches!(maturity, Maturity::Date(date) if *date <= today)
}

struct QuotePrice {
    value: f64,
    currency: Option<String>,
    source: &'static str,
    bid: Option<f64>,
    ask: Option<f64>,
    bid_size: Option<f64>,
    ask_size: Option<f64>,
    volume: Option<f64>,
}

enum AskParse {
    Positive {
        bid: Option<f64>,
        ask: f64,
        currency: Option<String>,
    },
    NotAvailable,
    Unknown,
}

fn buy_quote_price(product: &StructuredProduct) -> Option<QuotePrice> {
    if let Some(quote) = product.boursorama_quote.as_ref() {
        return sane_boursorama_ask(quote).map(|ask| QuotePrice {
            value: ask,
            currency: quote.currency.clone(),
            source: "boursorama_ask",
            bid: quote.bid,
            ask: Some(ask),
            bid_size: quote.bid_size,
            ask_size: quote.ask_size,
            volume: quote.total_volume,
        });
    }

    match parse_ask(&product.bid_ask) {
        AskParse::Positive { bid, ask, currency } => {
            if !sane_orderbook_quote(bid, ask) {
                return None;
            }
            return Some(QuotePrice {
                value: ask,
                currency,
                source: "euronext_ask",
                bid,
                ask: Some(ask),
                bid_size: None,
                ask_size: None,
                volume: None,
            });
        }
        AskParse::NotAvailable => return None,
        AskParse::Unknown => {}
    }

    product
        .last_price
        .value
        .filter(|value| *value > 0.0)
        .map(|value| QuotePrice {
            value,
            currency: product.last_price.currency.clone(),
            source: "last_unverified",
            bid: None,
            ask: None,
            bid_size: None,
            ask_size: None,
            volume: None,
        })
}

fn sane_boursorama_ask(quote: &crate::models::structured::BoursoramaQuote) -> Option<f64> {
    let ask = quote.ask.filter(|value| *value > 0.0)?;
    let bid = quote.bid.filter(|value| *value > 0.0)?;

    if ask < bid {
        return None;
    }

    if matches!(quote.bid_size, Some(size) if size <= 0.0) {
        return None;
    }
    if matches!(quote.ask_size, Some(size) if size <= 0.0) {
        return None;
    }

    let midpoint = (ask + bid) / 2.0;
    let spread = ask - bid;
    if midpoint > 0.0
        && spread > MIN_ABSOLUTE_PRICE_TOLERANCE
        && spread / midpoint > MAX_BOURSORAMA_RELATIVE_SPREAD
    {
        return None;
    }

    if let Some(last) = quote.last.filter(|value| *value > 0.0) {
        let deviation = (ask - last).abs();
        if deviation > MIN_ABSOLUTE_PRICE_TOLERANCE
            && deviation / last > MAX_BOURSORAMA_REFERENCE_DEVIATION
        {
            return None;
        }
    }

    Some(ask)
}

fn sane_orderbook_quote(bid: Option<f64>, ask: f64) -> bool {
    let Some(bid) = bid.filter(|value| *value > 0.0) else {
        return false;
    };
    if ask <= 0.0 || ask < bid {
        return false;
    }

    let midpoint = (bid + ask) / 2.0;
    if midpoint <= 0.0 {
        return false;
    }

    let spread = ask - bid;
    spread <= MIN_ABSOLUTE_PRICE_TOLERANCE
        || spread / midpoint <= MAX_EXECUTABLE_RELATIVE_SPREAD
}

#[cfg(test)]
fn parse_positive_ask(value: &str) -> Option<f64> {
    match parse_ask(value) {
        AskParse::Positive { ask, .. } => Some(ask),
        AskParse::NotAvailable | AskParse::Unknown => None,
    }
}

fn parse_ask(value: &str) -> AskParse {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "/" || trimmed == "-" {
        return AskParse::Unknown;
    }

    let has_sides = trimmed.contains('/');
    let bid_side = trimmed.split('/').next().unwrap_or(trimmed).trim();
    let ask_side = trimmed.split('/').next_back().unwrap_or(trimmed).trim();
    let bid = has_sides
        .then(|| bid_side.split_whitespace().find_map(parse_decimal))
        .flatten();
    let price = ask_side.split_whitespace().find_map(parse_decimal);
    match price {
        Some(price) if price > 0.0 && (!has_sides || bid.unwrap_or(0.0) > 0.0) => {
            AskParse::Positive {
                bid,
                ask: price,
                currency: parse_currency(ask_side).or_else(|| parse_currency(trimmed)),
            }
        }
        Some(_) if has_sides => AskParse::NotAvailable,
        Some(_) => AskParse::Unknown,
        None if has_sides && bid.unwrap_or(0.0) <= 0.0 => AskParse::NotAvailable,
        None => AskParse::Unknown,
    }
}

fn raw_intrinsic(direction: Direction, spot_price: f64, strike: f64) -> f64 {
    match direction {
        Direction::Call => (spot_price - strike).max(0.0),
        Direction::Put => (strike - spot_price).max(0.0),
        Direction::Unknown => 0.0,
    }
}

fn midpoint(bid: Option<f64>, ask: Option<f64>) -> Option<f64> {
    match (bid, ask) {
        (Some(bid), Some(ask)) if bid > 0.0 && ask > 0.0 && ask >= bid => {
            Some((bid + ask) / 2.0)
        }
        _ => None,
    }
}

fn spread_pct(bid: Option<f64>, ask: Option<f64>) -> Option<f64> {
    let mid = midpoint(bid, ask)?;
    let spread = ask? - bid?;
    (mid > 0.0).then_some(spread / mid * 100.0)
}

fn execution_status(quote_price: &QuotePrice) -> &'static str {
    match quote_price.source {
        "boursorama_ask" | "euronext_ask"
            if quote_price.bid.unwrap_or(0.0) > 0.0
                && quote_price.ask.unwrap_or(0.0) > 0.0 =>
        {
            "executable_bid_ask"
        }
        "last_unverified" => "last_unverified",
        _ => "not_executable",
    }
}

fn liquidity_score(quote_price: &QuotePrice) -> f64 {
    if execution_status(quote_price) != "executable_bid_ask" {
        return 0.0;
    }

    let size_score = match (quote_price.bid_size, quote_price.ask_size) {
        (Some(bid_size), Some(ask_size)) => (bid_size.min(ask_size) / 10.0).clamp(0.0, 70.0),
        _ => 45.0,
    };
    let volume_score = quote_price
        .volume
        .map(|volume| (volume / 100.0).clamp(0.0, 30.0))
        .unwrap_or(0.0);

    (size_score + volume_score).min(100.0)
}

fn data_quality_score(quote_price: &QuotePrice, spread_pct: Option<f64>) -> f64 {
    let mut score: f64 = match execution_status(quote_price) {
        "executable_bid_ask" => 100.0,
        "last_unverified" => 35.0,
        _ => 0.0,
    };

    if let Some(spread_pct) = spread_pct {
        if spread_pct > 5.0 {
            score -= 35.0;
        } else if spread_pct > 2.0 {
            score -= 15.0;
        }
    } else {
        score -= 25.0;
    }

    if matches!(quote_price.volume, Some(volume) if volume <= 0.0) {
        score -= 5.0;
    }

    score.clamp(0.0, 100.0)
}

fn barrier_distance_pct(
    direction: Direction,
    spot_in_reference_currency: f64,
    barrier: Option<f64>,
) -> Option<f64> {
    let barrier = barrier?;
    if spot_in_reference_currency <= 0.0 || barrier <= 0.0 {
        return None;
    }

    match direction {
        Direction::Call => Some((spot_in_reference_currency - barrier) / spot_in_reference_currency * 100.0),
        Direction::Put => Some((barrier - spot_in_reference_currency) / spot_in_reference_currency * 100.0),
        Direction::Unknown => None,
    }
}

fn effective_gearing(
    spot_in_reference_currency: f64,
    reference_currency: &str,
    price_currency: &str,
    last_price: f64,
    warrants_per_underlying: Option<f64>,
    fx_rates: &FxRateBook,
) -> Option<f64> {
    let parity = warrants_per_underlying?;
    if spot_in_reference_currency <= 0.0 || last_price <= 0.0 || parity <= 0.0 {
        return None;
    }

    let spot_in_price_currency =
        fx_rates.convert(spot_in_reference_currency, reference_currency, price_currency)?;
    Some(spot_in_price_currency / (last_price * parity))
}

fn option_kind(direction: Direction) -> Option<OptionKind> {
    match direction {
        Direction::Call => Some(OptionKind::Call),
        Direction::Put => Some(OptionKind::Put),
        Direction::Unknown => None,
    }
}

fn years_to_maturity(maturity: &Maturity, today: NaiveDate) -> Option<f64> {
    let Maturity::Date(date) = maturity else {
        return None;
    };
    let days = (*date - today).num_days();
    (days > 0).then_some(days as f64 / 365.0)
}

fn option_risk_free_rate() -> f64 {
    read_env_f64("OPTION_RISK_FREE_RATE").unwrap_or(0.045)
}

fn option_dividend_yield() -> f64 {
    read_env_f64("OPTION_DIVIDEND_YIELD").unwrap_or(0.005)
}

fn read_env_f64(key: &str) -> Option<f64> {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite())
}

fn volatility_signal(implied_volatility: Option<f64>, smile_median_iv: Option<f64>) -> &'static str {
    let Some(iv) = implied_volatility else {
        return "no_iv";
    };
    let Some(smile_iv) = smile_median_iv else {
        return "no_smile";
    };
    let threshold = read_env_f64("OPTION_IV_SIGNAL_THRESHOLD").unwrap_or(0.03);
    let gap = smile_iv - iv;

    if gap >= threshold {
        "iv_cheap"
    } else if gap <= -threshold {
        "iv_expensive"
    } else {
        "iv_neutral"
    }
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal));
    let len = values.len();
    if len == 0 {
        return 0.0;
    }
    if len % 2 == 1 {
        values[len / 2]
    } else {
        (values[len / 2 - 1] + values[len / 2]) / 2.0
    }
}

fn warrants_per_underlying(product: &StructuredProduct) -> Option<f64> {
    product
        .detail
        .as_ref()
        .and_then(|detail| detail.warrants_per_underlying)
}

fn parse_decimal(value: &str) -> Option<f64> {
    let normalized = value.trim().replace(',', ".");
    if normalized.is_empty() || normalized == "-" || normalized == "/" {
        return None;
    }
    normalized.parse().ok()
}

fn parse_currency(value: &str) -> Option<String> {
    value
        .split(|ch: char| !ch.is_ascii_alphabetic())
        .find(|token| matches!(*token, "EUR" | "USD" | "GBP" | "CHF"))
        .map(str::to_string)
}

fn distance_band(distance_to_spot_pct: Option<f64>, width_pct: f64) -> String {
    distance_to_spot_pct
        .filter(|value| value.is_finite())
        .map(|value| (value / width_pct).floor() as i64)
        .map(|bucket| bucket.to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    use crate::models::fx::{FxRate, FxRateBook};
    use crate::models::structured::{
        BoursoramaQuote, ListedPrice, Maturity, Moneyness, ProductDetail, StructuredProduct,
    };

    #[test]
    fn parses_positive_ask_from_bid_ask_strings() {
        assert_eq!(parse_positive_ask("/"), None);
        assert_eq!(parse_positive_ask("4.440 / 0.000"), None);
        assert_eq!(parse_positive_ask("3,620 / 0,000  EUR"), None);
        assert_eq!(parse_positive_ask("4.440 / 4.460"), Some(4.46));
        assert_eq!(parse_positive_ask("4,440 / EUR 4,460"), Some(4.46));
        assert_eq!(parse_positive_ask("3,140 / 3,150  EUR"), Some(3.15));
        assert_eq!(parse_positive_ask("EUR 4.460"), Some(4.46));
    }

    #[test]
    fn explicit_zero_ask_does_not_fallback_to_last_price() {
        let product = StructuredProduct {
            bid_ask: "3,620 / 0,000  EUR".to_string(),
            last_price: ListedPrice {
                currency: Some("EUR".to_string()),
                value: Some(3.62),
            },
            ..warrant_call("ZEROASK", 275.0, 3.62, Some(10.0), None)
        };

        assert!(buy_quote_price(&product).is_none());
    }

    #[test]
    fn positive_ask_keeps_its_quote_currency_separate_from_usd_strike() {
        let product = StructuredProduct {
            bid_ask: "3,140 / 3,150  EUR".to_string(),
            last_price: ListedPrice {
                currency: None,
                value: Some(3.14),
            },
            detail: Some(ProductDetail {
                quote_currency: None,
                strike_currency: Some("USD".to_string()),
                ..detail("Warrant Call", Some(275.0), None, None, Some(10.0))
            }),
            ..warrant_call("MIXEDFX", 275.0, 3.14, Some(10.0), None)
        };

        let quote_price = buy_quote_price(&product).expect("positive ask should be parsed");

        assert_eq!(quote_price.value, 3.15);
        assert_eq!(quote_price.currency.as_deref(), Some("EUR"));
    }

    #[test]
    fn vanilla_warrant_uses_break_even_premium_not_intrinsic_ratio() {
        let products = vec![
            warrant_call("D12QS", 280.0, 3.99, Some(10.0), None),
            warrant_call("PEER1", 280.0, 5.00, Some(10.0), None),
            warrant_call("PEER2", 280.0, 6.00, Some(10.0), None),
        ];

        let signals = rank_relative_value(&products, 300.23, "USD", &fx_book());
        let signal = signals
            .iter()
            .find(|signal| signal.symbol == "D12QS")
            .expect("D12QS should be ranked");

        assert_eq!(signal.metric_kind, "premium_pct");
        assert_eq!(signal.price_source, "last_unverified");
        assert_close(signal.intrinsic_per_product, 1.7393754, 1e-6);
        assert_close(signal.price_to_intrinsic, 2.294, 1e-3);
        assert_close(signal.relative_metric, 8.719, 1e-3);
    }

    #[test]
    fn boursorama_positive_ask_overrides_stale_last_price() {
        let products = vec![
            warrant_call("ASK1", 280.0, 3.99, Some(10.0), Some(4.50)),
            warrant_call("ASK2", 280.0, 5.00, Some(10.0), Some(5.20)),
            warrant_call("ASK3", 280.0, 6.00, Some(10.0), Some(6.30)),
        ];

        let signals = rank_relative_value(&products, 300.23, "USD", &fx_book());
        let signal = signals
            .iter()
            .find(|signal| signal.symbol == "ASK1")
            .expect("ASK1 should be ranked");

        assert_eq!(signal.price_source, "boursorama_ask");
        assert_eq!(signal.last_price, 4.50);
        assert!(signal.relative_metric > 8.719);
    }

    #[test]
    fn boursorama_zero_ask_excludes_bid_only_product() {
        let products = vec![
            warrant_call("48S1B", 250.0, 4.44, Some(10.0), Some(0.0)),
            warrant_call("PEER1", 250.0, 4.60, Some(10.0), None),
            warrant_call("PEER2", 250.0, 4.80, Some(10.0), None),
            warrant_call("PEER3", 250.0, 5.00, Some(10.0), None),
        ];

        let signals = rank_relative_value(&products, 300.23, "USD", &fx_book());

        assert!(!signals.iter().any(|signal| signal.symbol == "48S1B"));
        assert!(signals.iter().any(|signal| signal.symbol == "PEER1"));
    }

    #[test]
    fn boursorama_zero_bid_excludes_non_executable_product() {
        let products = vec![
            product_with_boursorama_bid_ask(
                "NOBID",
                "Warrant Call",
                Direction::Call,
                260.0,
                3.62,
                Some(0.0),
                Some(3.65),
            ),
            warrant_call("PEER1", 260.0, 3.70, Some(10.0), None),
            warrant_call("PEER2", 260.0, 3.80, Some(10.0), None),
            warrant_call("PEER3", 260.0, 3.90, Some(10.0), None),
        ];

        let signals = rank_relative_value(&products, 300.23, "USD", &fx_book());

        assert!(!signals.iter().any(|signal| signal.symbol == "NOBID"));
        assert!(signals.iter().any(|signal| signal.symbol == "PEER1"));
    }

    #[test]
    fn real_warrant_quote_computes_execution_and_volatility_indicators() {
        let products = vec![
            product_with_boursorama_bid_ask(
                "45S8B",
                "Warrant Call",
                Direction::Call,
                260.0,
                3.62,
                Some(3.64),
                Some(3.65),
            ),
            product_with_boursorama_bid_ask(
                "PEER1",
                "Warrant Call",
                Direction::Call,
                260.0,
                3.80,
                Some(3.79),
                Some(3.80),
            ),
            product_with_boursorama_bid_ask(
                "PEER2",
                "Warrant Call",
                Direction::Call,
                260.0,
                4.00,
                Some(3.99),
                Some(4.00),
            ),
        ];

        let signals = rank_relative_value(&products, 300.23, "USD", &fx_book());
        let signal = signals
            .iter()
            .find(|signal| signal.symbol == "45S8B")
            .expect("45S8B should be ranked");

        assert_eq!(signal.price_source, "boursorama_ask");
        assert_eq!(signal.execution_status, "executable_bid_ask");
        assert_close(signal.spread_pct.unwrap(), 0.2743, 1e-3);
        assert!(signal.data_quality_score >= 90.0);
        assert!(signal.implied_volatility.is_some());
        assert!(signal.smile_median_iv.is_some());
        assert_ne!(signal.volatility_signal, "no_iv");
    }

    #[test]
    fn boursorama_outlier_ask_is_rejected_for_financing_products() {
        let products = vec![
            product(
                "0SN8B",
                "Open-End Knock-Out Warrant Put",
                Direction::Put,
                Maturity::OpenEnd,
                319.32,
                Some(319.32),
                None,
                1.978,
                Some(10.0),
                Some(5.028),
            ),
            mini_future_put("PEER1", 319.32, 319.32, 1.78),
            mini_future_put("PEER2", 319.32, 319.32, 1.86),
            mini_future_put("PEER3", 319.32, 319.32, 1.94),
        ];

        let signals = rank_relative_value(&products, 300.23, "USD", &fx_book());

        assert!(!signals.iter().any(|signal| signal.symbol == "0SN8B"));
        assert!(
            signals.iter().any(|signal| signal.symbol == "PEER1"),
            "valid peers should still be ranked"
        );
    }

    #[test]
    fn matured_products_are_not_exploitable() {
        let maturity = Maturity::Date(NaiveDate::from_ymd_opt(2026, 5, 15).unwrap());
        assert!(is_matured_or_expired(
            &maturity,
            NaiveDate::from_ymd_opt(2026, 5, 15).unwrap()
        ));
        assert!(is_matured_or_expired(
            &maturity,
            NaiveDate::from_ymd_opt(2026, 5, 17).unwrap()
        ));
        assert!(!is_matured_or_expired(
            &maturity,
            NaiveDate::from_ymd_opt(2026, 5, 14).unwrap()
        ));
        assert!(!is_matured_or_expired(
            &Maturity::OpenEnd,
            NaiveDate::from_ymd_opt(2026, 5, 17).unwrap()
        ));
    }

    #[test]
    fn warrant_distance_buckets_are_finer_than_financing_buckets() {
        let deep_itm_call_distance = -16.73;
        let atm_call_distance = -0.08;

        assert_eq!(
            distance_band(Some(deep_itm_call_distance), 20.0),
            distance_band(Some(atm_call_distance), 20.0)
        );
        assert_ne!(
            distance_band(Some(deep_itm_call_distance), 5.0),
            distance_band(Some(atm_call_distance), 5.0)
        );
    }

    #[test]
    fn vanilla_warrant_without_parity_is_not_ranked() {
        let products = vec![
            warrant_call("NOPAR1", 280.0, 3.99, None, None),
            warrant_call("NOPAR2", 280.0, 5.00, None, None),
            warrant_call("NOPAR3", 280.0, 6.00, None, None),
        ];

        let signals = rank_relative_value(&products, 300.23, "USD", &fx_book());

        assert!(signals.is_empty());
    }

    #[test]
    fn mini_future_put_uses_financing_level_and_fx_adjusted_ratio() {
        let products = vec![
            mini_future_put("304TB", 303.93, 323.3388, 2.005),
            mini_future_put("PEER1", 303.93, 323.3388, 2.10),
            mini_future_put("PEER2", 303.93, 323.3388, 2.20),
        ];

        let signals = rank_relative_value(&products, 300.23, "USD", &fx_book());
        let signal = signals
            .iter()
            .find(|signal| signal.symbol == "304TB")
            .expect("304TB should be ranked");

        assert_eq!(signal.product_family, "mini_future");
        assert_eq!(signal.pricing_model, "financing_level");
        assert_eq!(signal.metric_kind, "price_to_intrinsic");
        assert_eq!(signal.strike, 323.3388);
        assert_eq!(signal.barrier, Some(303.93));
        assert_close(signal.price_to_intrinsic, 1.0091, 1e-3);
    }

    #[test]
    fn financing_peers_with_absurd_intrinsic_ratio_do_not_pollute_median() {
        let products = vec![
            open_end_put_with_bid_ask("VONTO", 1795.5689, 100.0, 2.20, 2.39),
            open_end_put_with_bid_ask("PEER1", 1788.7946, 100.0, 2.13, 2.32),
            open_end_put_with_bid_ask("PEER2", 1754.5609, 100.0, 1.79, 1.98),
            open_end_put_with_bid_ask("PEER3", 1694.8993, 100.0, 1.32, 1.50),
            open_end_put_with_bid_ask("STALE", 1620.0, 200.0, 23.98, 26.00),
        ];

        let signals = rank_relative_value(&products, 1575.5, "EUR", &fx_book());
        let signal = signals
            .iter()
            .find(|signal| signal.symbol == "VONTO")
            .expect("VONTO should be ranked against clean peers");

        assert_eq!(signal.peer_count, 4);
        assert!(signal.peer_median_relative_metric < 1.20);
        assert!(!signals.iter().any(|signal| signal.symbol == "STALE"));
    }

    #[test]
    fn euronext_quotes_with_extreme_spread_are_not_ranked() {
        let products = vec![
            open_end_put_with_bid_ask("OK1", 1795.5689, 100.0, 2.20, 2.39),
            open_end_put_with_bid_ask("OK2", 1788.7946, 100.0, 2.13, 2.32),
            open_end_put_with_bid_ask("OK3", 1754.5609, 100.0, 1.79, 1.98),
            open_end_put_with_bid_ask("WIDE", 1711.86, 100.0, 1.00, 1.50),
        ];

        let signals = rank_relative_value(&products, 1575.5, "EUR", &fx_book());

        assert!(!signals.iter().any(|signal| signal.symbol == "WIDE"));
    }

    #[test]
    fn missing_detail_or_halted_status_is_excluded() {
        let without_detail = StructuredProduct {
            detail: None,
            ..warrant_call("NODETAIL", 280.0, 3.99, Some(10.0), None)
        };
        let halted = StructuredProduct {
            detail: Some(ProductDetail {
                trading_status: Some("HAL".to_string()),
                ..detail("Warrant Call", Some(280.0), None, None, Some(10.0))
            }),
            ..warrant_call("HALTED", 280.0, 4.10, Some(10.0), None)
        };
        let products = vec![
            without_detail,
            halted,
            warrant_call("OK1", 280.0, 5.00, Some(10.0), None),
            warrant_call("OK2", 280.0, 6.00, Some(10.0), None),
        ];

        let signals = rank_relative_value(&products, 300.23, "USD", &fx_book());

        assert!(!signals.iter().any(|signal| signal.symbol == "NODETAIL"));
        assert!(!signals.iter().any(|signal| signal.symbol == "HALTED"));
        assert!(signals.is_empty(), "only two usable peers remain, below MIN_PEERS");
    }

    #[test]
    fn standalone_signals_do_not_require_relative_value_peers() {
        let products = vec![product_with_boursorama_bid_ask(
            "SOLO",
            "Warrant Call",
            Direction::Call,
            280.0,
            3.99,
            Some(3.98),
            Some(4.00),
        )];

        let relative_signals = rank_relative_value(&products, 300.23, "USD", &fx_book());
        let standalone_signals =
            standalone_product_signals(&products, 300.23, "USD", &fx_book());

        assert!(relative_signals.is_empty());
        assert_eq!(standalone_signals.len(), 1);
        assert_eq!(standalone_signals[0].symbol, "SOLO");
        assert_eq!(standalone_signals[0].peer_count, 1);
        assert_eq!(standalone_signals[0].execution_status, "executable_bid_ask");
        assert!(standalone_signals[0].implied_volatility.is_some());
    }

    #[test]
    fn scenario_warrant_signals_keep_otm_vanilla_warrants() {
        let mut otm_call = product_with_boursorama_bid_ask(
            "OTM",
            "Warrant Call",
            Direction::Call,
            330.0,
            0.85,
            Some(0.84),
            Some(0.86),
        );
        otm_call.moneyness = Moneyness::OutOfTheMoney;

        let products = vec![otm_call];

        let standalone_signals =
            standalone_product_signals(&products, 300.23, "USD", &fx_book());
        let scenario_signals = scenario_warrant_signals(&products, 300.23, "USD", &fx_book());

        assert!(
            standalone_signals.is_empty(),
            "relative-value standalone keeps only products with current intrinsic value"
        );
        assert_eq!(scenario_signals.len(), 1);
        assert_eq!(scenario_signals[0].symbol, "OTM");
        assert_eq!(scenario_signals[0].pricing_model, "warrant_intrinsic");
        assert_eq!(scenario_signals[0].execution_status, "executable_bid_ask");
        assert_eq!(scenario_signals[0].raw_intrinsic, 0.0);
        assert!(scenario_signals[0].implied_volatility.is_some());
    }

    fn fx_book() -> FxRateBook {
        let mut book = FxRateBook::default();
        book.add(FxRate {
            from: "USD".to_string(),
            to: "EUR".to_string(),
            rate: 0.8598,
            source_ticker: "EURUSD=X".to_string(),
        });
        book
    }

    fn warrant_call(
        symbol: &str,
        strike: f64,
        last_price: f64,
        parity: Option<f64>,
        boursorama_ask: Option<f64>,
    ) -> StructuredProduct {
        product(
            symbol,
            "Warrant Call",
            Direction::Call,
            Maturity::Date(NaiveDate::from_ymd_opt(2027, 3, 19).unwrap()),
            strike,
            None,
            None,
            last_price,
            parity,
            boursorama_ask,
        )
    }

    fn mini_future_put(
        symbol: &str,
        barrier: f64,
        financing_level: f64,
        last_price: f64,
    ) -> StructuredProduct {
        product(
            symbol,
            "Mini-Future Short",
            Direction::Put,
            Maturity::OpenEnd,
            barrier,
            Some(financing_level),
            Some(barrier),
            last_price,
            Some(10.0),
            None,
        )
    }

    fn open_end_put_with_bid_ask(
        symbol: &str,
        reference: f64,
        parity: f64,
        bid: f64,
        ask: f64,
    ) -> StructuredProduct {
        let mut product = product(
            symbol,
            "Open-End Knock-Out Warrant Put",
            Direction::Put,
            Maturity::OpenEnd,
            reference,
            Some(reference),
            Some(reference),
            ask,
            Some(parity),
            None,
        );
        product.bid_ask = format!("{bid:.4} / {ask:.4}");
        product.underlying = "HERMES INTL".to_string();
        product.last_price.currency = Some("EUR".to_string());
        if let Some(detail) = product.detail.as_mut() {
            detail.quote_currency = Some("EUR".to_string());
            detail.strike_currency = Some("EUR".to_string());
            detail.second_strike_currency = Some("EUR".to_string());
            detail.underlying_designation = Some("HERMES INTL".to_string());
            detail.underlying_group_name = Some("Hermes International".to_string());
            detail.underlying_isin = Some("FR0000052292".to_string());
        }
        product
    }

    fn product_with_boursorama_bid_ask(
        symbol: &str,
        product_type: &str,
        direction: Direction,
        strike: f64,
        last_price: f64,
        bid: Option<f64>,
        ask: Option<f64>,
    ) -> StructuredProduct {
        let mut product = product(
            symbol,
            product_type,
            direction,
            Maturity::Date(NaiveDate::from_ymd_opt(2026, 6, 18).unwrap()),
            strike,
            None,
            None,
            last_price,
            Some(10.0),
            None,
        );
        product.boursorama_quote = Some(BoursoramaQuote {
            symbol: format!("1rP{symbol}"),
            url: format!("https://example.test/{symbol}"),
            currency: Some("EUR".to_string()),
            last: Some(last_price),
            previous_close: None,
            high: None,
            low: None,
            total_volume: None,
            variation: None,
            trade_date: None,
            bid,
            ask,
            bid_size: bid.map(|value| if value > 0.0 { 100.0 } else { 0.0 }),
            ask_size: ask.map(|value| if value > 0.0 { 100.0 } else { 0.0 }),
        });
        product
    }

    #[allow(clippy::too_many_arguments)]
    fn product(
        symbol: &str,
        product_type: &str,
        direction: Direction,
        maturity: Maturity,
        strike: f64,
        second_strike: Option<f64>,
        lower_threshold: Option<f64>,
        last_price: f64,
        parity: Option<f64>,
        boursorama_ask: Option<f64>,
    ) -> StructuredProduct {
        StructuredProduct {
            symbol: symbol.to_string(),
            boursorama_symbol: format!("1rP{symbol}"),
            boursorama_url: format!("https://example.test/{symbol}"),
            yahoo_symbol: None,
            isin: None,
            mic: None,
            name: Some(product_type.to_string()),
            underlying: "APPLE".to_string(),
            product_type: product_type.to_string(),
            direction,
            strike: Some(strike),
            maturity,
            bid_ask: "/".to_string(),
            last_price: ListedPrice {
                currency: Some("EUR".to_string()),
                value: Some(last_price),
            },
            last_trade_time: String::new(),
            distance_to_spot_pct: None,
            moneyness: Moneyness::InTheMoney,
            detail_path: None,
            detail: Some(detail(
                product_type,
                Some(strike),
                second_strike,
                lower_threshold,
                parity,
            )),
            boursorama_quote: boursorama_ask.map(|ask| BoursoramaQuote {
                symbol: format!("1rP{symbol}"),
                url: format!("https://example.test/{symbol}"),
                currency: Some("EUR".to_string()),
                last: Some(last_price),
                previous_close: None,
                high: None,
                low: None,
                total_volume: None,
                variation: None,
                trade_date: None,
                bid: Some(last_price),
                ask: Some(ask),
                bid_size: Some(100.0),
                ask_size: if ask > 0.0 { Some(100.0) } else { Some(0.0) },
            }),
        }
    }

    fn detail(
        product_type: &str,
        strike: Option<f64>,
        second_strike: Option<f64>,
        lower_threshold: Option<f64>,
        parity: Option<f64>,
    ) -> ProductDetail {
        ProductDetail {
            short_name: None,
            long_name: None,
            issuer_name: Some("TEST ISSUER".to_string()),
            quote_currency: Some("EUR".to_string()),
            issue_date: None,
            issue_price: None,
            issue_price_currency: None,
            introduction_date: None,
            parity_warrant_underlying: None,
            parity_underlying_warrant: parity,
            parity_first_warrant_underlying: parity.map(|value| 1.0 / value),
            warrants_per_underlying: parity,
            strike_price: strike,
            strike_currency: Some("USD".to_string()),
            second_strike_price: second_strike,
            second_strike_currency: Some("USD".to_string()),
            leverage_level: None,
            lower_threshold,
            marketing_product_name: Some(product_type.to_string()),
            underlying_designation: Some("APPLE".to_string()),
            underlying_group_name: Some("Apple Computer".to_string()),
            underlying_isin: Some("US0378331005".to_string()),
            kid_url: None,
            opening_time: Some("08:00".to_string()),
            closing_time: Some("22:00".to_string()),
            trading_open_time: None,
            trading_close_time: None,
            trading_lot: None,
            number_of_shares: None,
            price_multiplier: None,
            trading_status: Some("CLO".to_string()),
            quotation_state: Some("AUT".to_string()),
            last_price: None,
            last_quantity: None,
            open_price: None,
            close_price: None,
            previous_close_price: None,
            high_price: None,
            low_price: None,
            valorization: None,
            valorization_datetime: None,
            traded_quantity: Some(0.0),
            traded_amount: None,
            trade_count: Some(0),
            last_update: None,
            last_quote_datetime: None,
            last_trade_type: None,
            halt_reason: None,
            quality: None,
            listed: Some(true),
        }
    }

    fn assert_close(actual: f64, expected: f64, tolerance: f64) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "actual={actual}, expected={expected}, tolerance={tolerance}"
        );
    }
}
