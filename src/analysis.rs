use std::cmp::Ordering;
use std::collections::BTreeSet;

use anyhow::{Context, Result};
use reqwest::Client;

use crate::api::boursorama::fetch_boursorama_quote_for_product;
use crate::api::boursorama_discover::discover_boursorama_by_underlying;
use crate::api::discover::{discover_by_underlying, fetch_instrument_detail};
use crate::api::fetch::fetch_warrant;
use crate::api::fx::fetch_fx_rate;
use crate::models::fx::FxRateBook;
use crate::models::structured::{Direction, StructuredProduct};
use crate::models::warrant::WarrantSnapshot;
use crate::trace;

#[derive(Debug)]
pub struct MarketBase {
    pub underlying_quote: WarrantSnapshot,
    pub fx_rates: FxRateBook,
    pub products: Vec<StructuredProduct>,
}

pub async fn build_market_base(
    client: &Client,
    underlying: &str,
    underlying_ticker: &str,
    limit: Option<usize>,
) -> Result<MarketBase> {
    trace::log(format!(
        "market_base: fetching spot ticker={underlying_ticker}"
    ));
    let underlying_quote = fetch_warrant(client, underlying_ticker)
        .await
        .with_context(|| format!("Impossible de recuperer le prix spot pour {underlying_ticker}"))?;
    trace::log(format!(
        "market_base: spot ready {} {:.4} {}",
        underlying_quote.ticker, underlying_quote.price, underlying_quote.currency
    ));
    trace::log(format!(
        "market_base: discovering products underlying={underlying} limit={limit:?}"
    ));
    let mut raw_products = discover_by_underlying(client, underlying, limit).await?;
    if raw_products.is_empty() {
        trace::log(format!(
            "market_base: Euronext discovery returned 0, trying Boursorama fallback for {underlying}"
        ));
        raw_products = discover_boursorama_by_underlying(client, underlying, limit).await?;
        trace::log(format!(
            "market_base: Boursorama fallback done raw_products={}",
            raw_products.len()
        ));
    }
    trace::log(format!(
        "market_base: discovery done raw_products={}",
        raw_products.len()
    ));
    let mut products = Vec::with_capacity(raw_products.len());
    let enrich_boursorama = std::env::var("BOURSORAMA_ENRICH")
        .map(|value| matches!(value.as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false);
    trace::log(format!(
        "market_base: enriching details products={} boursorama_enrich={enrich_boursorama}",
        raw_products.len()
    ));

    let total = raw_products.len();
    let mut detail_ok = 0usize;
    let mut detail_failed = 0usize;
    let mut boursorama_ok = 0usize;
    let mut boursorama_failed = 0usize;
    for (index, product) in raw_products.into_iter().enumerate() {
        trace::log_progress(index + 1, total, "market_base detail", &product.symbol);
        let detail = match fetch_instrument_detail(client, &product).await {
            Ok(detail) => {
                detail_ok += 1;
                Some(detail)
            }
            Err(err) => {
                detail_failed += 1;
                trace::log(format!(
                    "market_base detail: {} failed: {err:#}",
                    product.symbol
                ));
                None
            }
        };
        let mut structured = match detail {
            Some(detail) => StructuredProduct::from_euronext(product, underlying_quote.price)
                .with_detail(detail),
            None => StructuredProduct::from_euronext(product, underlying_quote.price),
        };
        if enrich_boursorama {
            match fetch_boursorama_quote_for_product(client, &structured).await {
                Ok(quote) => {
                    boursorama_ok += 1;
                    structured = structured.with_boursorama_quote(quote);
                }
                Err(err) => {
                    boursorama_failed += 1;
                    trace::log(format!(
                        "market_base Boursorama enrich: {} failed: {err:#}",
                        structured.symbol
                    ));
                }
            }
        }
        products.push(structured);
    }
    trace::log(format!(
        "market_base: details done ok={detail_ok} failed={detail_failed} boursorama_ok={boursorama_ok} boursorama_failed={boursorama_failed}"
    ));

    trace::log("market_base: building FX rate book");
    let fx_rates = build_fx_rate_book(client, &underlying_quote.currency, &products).await;
    trace::log("market_base: sorting products");
    sort_products(&mut products);
    trace::log("market_base: ready");

    Ok(MarketBase {
        underlying_quote,
        fx_rates,
        products,
    })
}

async fn build_fx_rate_book(
    client: &Client,
    underlying_currency: &str,
    products: &[StructuredProduct],
) -> FxRateBook {
    let mut pairs = BTreeSet::new();
    for product in products {
        let strike_currency = product
            .detail
            .as_ref()
            .and_then(|detail| detail.strike_currency.as_deref())
            .unwrap_or(underlying_currency);
        let price_currency = product
            .last_price
            .currency
            .as_deref()
            .or_else(|| {
                product
                    .detail
                    .as_ref()
                    .and_then(|detail| detail.quote_currency.as_deref())
            })
            .unwrap_or(underlying_currency);

        if !underlying_currency.eq_ignore_ascii_case(strike_currency) {
            pairs.insert((underlying_currency.to_string(), strike_currency.to_string()));
        }
        if !strike_currency.eq_ignore_ascii_case(price_currency) {
            pairs.insert((strike_currency.to_string(), price_currency.to_string()));
        }
    }

    let mut book = FxRateBook::default();
    for (from, to) in pairs {
        trace::log(format!("fx: fetching {from}->{to}"));
        if let Some(rate) = fetch_fx_rate(client, &from, &to).await {
            trace::log(format!(
                "fx: ready {}->{} rate={:.6} source={}",
                rate.from, rate.to, rate.rate, rate.source_ticker
            ));
            book.add(rate);
        } else {
            trace::log(format!("fx: missing {from}->{to}"));
        }
    }
    book
}

fn sort_products(products: &mut [StructuredProduct]) {
    products.sort_by(|left, right| {
        direction_rank(left.direction)
            .cmp(&direction_rank(right.direction))
            .then_with(|| left.maturity.sort_key().cmp(&right.maturity.sort_key()))
            .then_with(|| cmp_option_f64(left.strike, right.strike))
            .then_with(|| cmp_option_f64(left.last_price.value, right.last_price.value))
            .then_with(|| left.symbol.cmp(&right.symbol))
    });
}

fn direction_rank(direction: Direction) -> u8 {
    match direction {
        Direction::Call => 0,
        Direction::Put => 1,
        Direction::Unknown => 2,
    }
}

fn cmp_option_f64(left: Option<f64>, right: Option<f64>) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => left.partial_cmp(&right).unwrap_or(Ordering::Equal),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}
