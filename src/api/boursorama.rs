use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::Value;

use crate::models::structured::BoursoramaQuote;
use crate::models::structured::StructuredProduct;

pub fn boursorama_symbol(symbol: &str) -> String {
    format!("1rP{symbol}")
}

pub fn boursorama_url(symbol: &str) -> String {
    format!(
        "https://www.boursorama.com/bourse/produits-de-bourse/cours/{}",
        boursorama_symbol(symbol)
    )
}

pub async fn fetch_boursorama_quote(
    client: &Client,
    symbol: &str,
) -> Result<BoursoramaQuote> {
    let url = boursorama_url(symbol);
    let boursorama_symbol = boursorama_symbol(symbol);
    fetch_boursorama_quote_by_symbol(client, symbol, &boursorama_symbol, &url).await
}

pub async fn fetch_boursorama_quote_for_product(
    client: &Client,
    product: &StructuredProduct,
) -> Result<BoursoramaQuote> {
    fetch_boursorama_quote_by_symbol(
        client,
        &product.symbol,
        &product.boursorama_symbol,
        &product.boursorama_url,
    )
    .await
}

pub async fn fetch_boursorama_quote_by_symbol(
    client: &Client,
    symbol: &str,
    boursorama_symbol: &str,
    url: &str,
) -> Result<BoursoramaQuote> {
    let html = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("Erreur HTTP Boursorama pour {symbol}"))?
        .error_for_status()
        .with_context(|| format!("Boursorama refuse {symbol}"))?
        .text()
        .await
        .with_context(|| format!("Impossible de lire la page Boursorama pour {symbol}"))?;

    parse_boursorama_quote(symbol, boursorama_symbol, url, &html)
}

fn parse_boursorama_quote(
    _symbol: &str,
    boursorama_symbol: &str,
    url: &str,
    html: &str,
) -> Result<BoursoramaQuote> {
    let marker = format!("data-ist=\"{boursorama_symbol}\" data-ist-orderbook");
    let start = html
        .find(&marker)
        .or_else(|| html.find(&format!("data-ist=\"{boursorama_symbol}\"")))
        .or_else(|| html.find(&format!("data-ist='{boursorama_symbol}'")))
        .context("Symbole Boursorama introuvable dans la page")?;
    let fragment = &html[start..];
    let init_marker = "data-ist-init=\"";
    let init_start = fragment
        .find(init_marker)
        .context("JSON data-ist-init introuvable")?
        + init_marker.len();
    let init_end = fragment[init_start..]
        .find('"')
        .context("JSON data-ist-init incomplet")?
        + init_start;
    let json = decode_entities(&fragment[init_start..init_end]);
    let value: Value = serde_json::from_str(&json).context("JSON Boursorama invalide")?;

    let first_book_line = value
        .get("orderbook")
        .and_then(|orderbook| orderbook.get("lines"))
        .and_then(Value::as_array)
        .and_then(|lines| lines.first());

    Ok(BoursoramaQuote {
        symbol: boursorama_symbol.to_string(),
        url: url.to_string(),
        currency: parse_quote_currency(fragment),
        last: value.get("last").and_then(value_as_f64),
        previous_close: value.get("previousClose").and_then(value_as_f64),
        high: value.get("high").and_then(value_as_f64),
        low: value.get("low").and_then(value_as_f64),
        total_volume: value.get("totalVolume").and_then(value_as_f64),
        variation: value.get("variation").and_then(value_as_f64),
        trade_date: value
            .get("tradeDate")
            .and_then(Value::as_str)
            .map(str::to_string),
        bid: first_book_line.and_then(|line| line.get("bid")).and_then(value_as_f64),
        ask: first_book_line.and_then(|line| line.get("ask")).and_then(value_as_f64),
        bid_size: first_book_line
            .and_then(|line| line.get("bidSize"))
            .and_then(value_as_f64),
        ask_size: first_book_line
            .and_then(|line| line.get("askSize"))
            .and_then(value_as_f64),
    })
}

fn value_as_f64(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(parse_decimal))
}

fn parse_decimal(value: &str) -> Option<f64> {
    let normalized = value.trim().replace(',', ".");
    if normalized.is_empty() || normalized == "-" || normalized == "/" {
        return None;
    }
    normalized.parse().ok()
}

fn parse_quote_currency(fragment: &str) -> Option<String> {
    ["EUR", "USD", "GBP", "CHF"]
        .iter()
        .find(|currency| {
            fragment.contains(&format!(" {currency}"))
                || fragment.contains(&format!(">{currency}<"))
                || fragment.contains(&format!("&nbsp;{currency}"))
        })
        .map(|currency| (*currency).to_string())
}

fn decode_entities(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#039;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_orderbook_bid_ask_and_zero_ask() {
        let html = r#"
            <div
              data-ist="1rP48S1B"
              data-ist-orderbook
              data-ist-init="{&quot;last&quot;:4.44,&quot;previousClose&quot;:4.25,&quot;high&quot;:4.69,&quot;low&quot;:3.92,&quot;totalVolume&quot;:0,&quot;tradeDate&quot;:&quot;2026-05-15T17:21:00&quot;,&quot;orderbook&quot;:{&quot;lines&quot;:[{&quot;bid&quot;:4.44,&quot;ask&quot;:0.0,&quot;bidSize&quot;:1200,&quot;askSize&quot;:0}]}}">
            </div>
        "#;

        let quote = parse_boursorama_quote("48S1B", "1rP48S1B", "https://example.test", html).unwrap();

        assert_eq!(quote.symbol, "1rP48S1B");
        assert_eq!(quote.last, Some(4.44));
        assert_eq!(quote.bid, Some(4.44));
        assert_eq!(quote.ask, Some(0.0));
        assert_eq!(quote.ask_size, Some(0.0));
    }

    #[test]
    fn parses_string_numbers_and_currency_from_boursorama_fragment() {
        let html = r#"
            <div
              data-ist="1rPTEST"
              data-ist-orderbook
              data-ist-init="{&quot;last&quot;:&quot;3,145&quot;,&quot;orderbook&quot;:{&quot;lines&quot;:[{&quot;bid&quot;:&quot;3,140&quot;,&quot;ask&quot;:&quot;3,150&quot;,&quot;bidSize&quot;:&quot;500&quot;,&quot;askSize&quot;:&quot;800&quot;}]}}">
              3,140 / 3,150 EUR
              Strike 275 USD
            </div>
        "#;

        let quote = parse_boursorama_quote("TEST", "1rPTEST", "https://example.test", html).unwrap();

        assert_eq!(quote.currency.as_deref(), Some("EUR"));
        assert_eq!(quote.bid, Some(3.14));
        assert_eq!(quote.ask, Some(3.15));
        assert_eq!(quote.ask_size, Some(800.0));
    }

    #[test]
    fn fails_when_symbol_marker_is_missing() {
        let err = parse_boursorama_quote(
            "48S1B",
            "1rP48S1B",
            "https://example.test",
            r#"<div data-ist="1rPOTHER" data-ist-init="{}"></div>"#,
        )
        .unwrap_err();

        assert!(err.to_string().contains("Symbole Boursorama introuvable"));
    }
}
