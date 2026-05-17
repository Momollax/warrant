use std::collections::BTreeMap;

use anyhow::{Context, Result};
use reqwest::Client;

use crate::models::euronext::{
    EuronextDirectoryResponse, EuronextProduct, InstrumentDetail, InstrumentDetailResponse,
};
use crate::trace;

const EURONEXT_DIRECTORY_URL: &str = "https://live.euronext.com/en/product_directory/data/warrants-all-markets?mics=ENXL%2CETLX%2CSEDX%2CXAMS%2CXBRU%2CXLIS%2CXMLI%2CXPAR";
const DISPLAY_DATAPOINTS: &str =
    "symbol,underlyingInstrument,productName,strike,maturity,ask,lastPrice,lastTradeTime";
const PAGE_SIZE: usize = 100;

pub async fn discover_by_underlying(
    client: &Client,
    underlying: &str,
    limit: Option<usize>,
) -> Result<Vec<EuronextProduct>> {
    let mut products = Vec::new();
    let mut start = 0_usize;
    let requested = limit.unwrap_or(usize::MAX);

    loop {
        let page_len = PAGE_SIZE.min(requested.saturating_sub(products.len()));
        if page_len == 0 {
            break;
        }

        trace::log(format!(
            "discover: fetching page start={start} length={page_len} underlying={underlying}"
        ));
        let page = fetch_page(client, underlying, start, page_len).await?;
        let total = page.total_display_records;
        let row_count = page.rows.len();
        trace::log(format!(
            "discover: page received rows={row_count} total={total} collected_before={}",
            products.len()
        ));

        for row in page.rows {
            products.push(parse_row(row));
            if products.len() >= requested {
                break;
            }
        }

        start += row_count;
        if row_count == 0 || start >= total || products.len() >= requested {
            break;
        }
    }

    Ok(products)
}

async fn fetch_page(
    client: &Client,
    underlying: &str,
    start: usize,
    length: usize,
) -> Result<EuronextDirectoryResponse> {
    let mut form = BTreeMap::new();
    form.insert("draw".to_string(), "1".to_string());
    form.insert("start".to_string(), start.to_string());
    form.insert("length".to_string(), length.to_string());
    form.insert("iDisplayStart".to_string(), start.to_string());
    form.insert("iDisplayLength".to_string(), length.to_string());
    form.insert("sSortField".to_string(), "symbol".to_string());
    form.insert("sSortDir_0".to_string(), "asc".to_string());
    form.insert(
        "args[underlyingInstrument]".to_string(),
        underlying.to_lowercase(),
    );
    form.insert(
        "args[display_datapoints]".to_string(),
        DISPLAY_DATAPOINTS.to_string(),
    );
    form.insert("args[initialLetter]".to_string(), String::new());

    client
        .post(EURONEXT_DIRECTORY_URL)
        .header("Referer", "https://live.euronext.com/en/products/structured-products/list")
        .header("X-Requested-With", "XMLHttpRequest")
        .form(&form)
        .send()
        .await
        .context("Erreur HTTP lors de la recherche Euronext")?
        .error_for_status()
        .context("Euronext a refuse la recherche de produits structures")?
        .json()
        .await
        .context("Impossible de parser la reponse JSON Euronext")
}

fn parse_row(row: Vec<String>) -> EuronextProduct {
    let symbol_html = row.first().map(String::as_str).unwrap_or_default();
    let href = extract_attr(symbol_html, "href");
    let (isin, mic) = href
        .as_deref()
        .and_then(parse_isin_mic)
        .unwrap_or((None, None));
    let symbol = strip_html(symbol_html);
    let yahoo_symbol = mic.as_deref().and_then(|m| yahoo_symbol(&symbol, m));

    EuronextProduct {
        symbol,
        yahoo_symbol,
        isin,
        mic,
        name: extract_attr(symbol_html, "data-order"),
        underlying: clean_cell(row.get(1)),
        product_type: clean_cell(row.get(2)),
        strike: clean_cell(row.get(3)),
        maturity: clean_cell(row.get(4)),
        bid_ask: clean_cell(row.get(5)),
        last_price: clean_cell(row.get(6)),
        last_trade_time: clean_cell(row.get(7)),
        detail_path: href,
    }
}

pub async fn fetch_instrument_detail(
    client: &Client,
    product: &EuronextProduct,
) -> Result<InstrumentDetail> {
    let isin = product
        .isin
        .as_deref()
        .context("Produit Euronext sans ISIN")?;
    let mic = product
        .mic
        .as_deref()
        .context("Produit Euronext sans MIC")?;
    let url = format!(
        "https://gateway.euronext.com/api/instrumentDetail?code={isin}&codification=ISIN&exchCode={mic}&sessionQuality=RT&view=FULL&authKey=256f0720127269acfcb390b5adef10c242469c41afd08426744324bee3e2d75a"
    );

    let response: InstrumentDetailResponse = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("Erreur HTTP instrumentDetail pour {}", product.symbol))?
        .error_for_status()
        .with_context(|| format!("Euronext instrumentDetail refuse {}", product.symbol))?
        .json()
        .await
        .with_context(|| format!("Impossible de parser instrumentDetail pour {}", product.symbol))?;

    Ok(response.instr)
}

fn clean_cell(cell: Option<&String>) -> String {
    cell.map(|value| strip_html(value)).unwrap_or_default()
}

fn parse_isin_mic(href: &str) -> Option<(Option<String>, Option<String>)> {
    let slug = href.rsplit('/').next()?;
    let (isin, mic) = slug.rsplit_once('-')?;
    Some((Some(isin.to_string()), Some(mic.to_string())))
}

fn yahoo_symbol(symbol: &str, mic: &str) -> Option<String> {
    let suffix = match mic {
        "XPAR" | "ENXL" => ".PA",
        "XAMS" => ".AS",
        "XBRU" => ".BR",
        "XLIS" => ".LS",
        "XMLI" | "ETLX" | "SEDX" => ".MI",
        _ => return None,
    };
    Some(format!("{symbol}{suffix}"))
}

fn extract_attr(fragment: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=");
    let start = fragment.find(&needle)? + needle.len();
    let quote = fragment[start..].chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }

    let value_start = start + quote.len_utf8();
    let value_end = fragment[value_start..].find(quote)? + value_start;
    Some(decode_entities(&fragment[value_start..value_end]))
}

fn strip_html(input: &str) -> String {
    let mut output = String::new();
    let mut in_tag = false;

    for ch in input.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                output.push(' ');
            }
            _ if !in_tag => output.push(ch),
            _ => {}
        }
    }

    decode_entities(&output)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
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
