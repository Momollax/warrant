use anyhow::{Context, Result};
use base64::{engine::general_purpose, Engine as _};
use reqwest::Client;
use std::collections::HashSet;

use crate::models::euronext::EuronextProduct;

const BOURSORAMA_BASE_URL: &str = "https://www.boursorama.com";

pub async fn discover_boursorama_by_underlying(
    client: &Client,
    underlying: &str,
    limit: Option<usize>,
) -> Result<Vec<EuronextProduct>> {
    let Some(boursorama_underlying) = boursorama_underlying_name(underlying) else {
        return Ok(Vec::new());
    };

    let encoded = boursorama_underlying.replace(' ', "+");
    let urls = [
        format!(
            "{BOURSORAMA_BASE_URL}/bourse/produits-de-bourse/levier/warrants/resultats?warrant_filter%5BunderlyingName%5D%5B0%5D={encoded}"
        ),
        format!(
            "{BOURSORAMA_BASE_URL}/bourse/produits-de-bourse/levier/turbos/resultats?turbos_filter%5BunderlyingName%5D%5B0%5D={encoded}"
        ),
    ];

    let requested = limit.unwrap_or(usize::MAX);
    let mut seen = HashSet::new();
    let mut products = Vec::new();
    for url in urls {
        let html = client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("Erreur HTTP Boursorama discovery pour {underlying}"))?
            .error_for_status()
            .with_context(|| format!("Boursorama refuse discovery pour {underlying}"))?
            .text()
            .await
            .with_context(|| format!("Impossible de lire Boursorama discovery pour {underlying}"))?;

        for product in parse_boursorama_products(&html) {
            let key = product
                .isin
                .clone()
                .unwrap_or_else(|| product.symbol.clone());
            if seen.insert(key) {
                products.push(product);
                if products.len() >= requested {
                    return Ok(products);
                }
            }
        }
    }

    Ok(products)
}

fn boursorama_underlying_name(underlying: &str) -> Option<&'static str> {
    match underlying.to_ascii_lowercase().as_str() {
        "hermes" | "hermès" | "rms" | "rms.pa" => Some("HERMES INTL"),
        _ => None,
    }
}

fn parse_boursorama_products(html: &str) -> Vec<EuronextProduct> {
    html.split("<tr class=\"c-table__row\"")
        .filter_map(parse_boursorama_row)
        .collect()
}

fn parse_boursorama_row(row: &str) -> Option<EuronextProduct> {
    if !row.contains("Accéder à la fiche") && !row.contains("Acc&eacute;der &agrave; la fiche") {
        return None;
    }

    let data_rel = extract_attr(row, "data-rel")?;
    let path = decode_data_rel(&data_rel)?;
    let boursorama_symbol = path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|value| !value.is_empty())?
        .to_string();
    let isin = strip_html(extract_between(row, ">", "</span>")?).trim().to_string();
    if isin.is_empty() {
        return None;
    }

    let cells = table_cells(row);
    if cells.len() < 10 {
        return None;
    }

    let product_type = cells.get(2).cloned().unwrap_or_default();
    let underlying = cells
        .get(3)
        .map(|value| value.replace('…', "TL"))
        .unwrap_or_else(|| "HERMES INTL".to_string());
    let strike = cells.get(4).cloned().unwrap_or_default();
    let maturity = cells
        .get(5)
        .map(|value| normalize_boursorama_maturity(value))
        .unwrap_or_default();
    let bid = cells.get(7).cloned().unwrap_or_default();
    let ask = cells.get(8).cloned().unwrap_or_default();
    let last_price = preferred_boursorama_price(&bid, &ask);
    let title = extract_attr(row, "title")
        .map(|value| {
            value
                .replace("Accéder à la fiche de ", "")
                .replace("Acc&eacute;der &agrave; la fiche de ", "")
        })
        .filter(|value| !value.is_empty());

    Some(EuronextProduct {
        symbol: boursorama_symbol.clone(),
        yahoo_symbol: None,
        isin: Some(isin),
        mic: Some("XPAR".to_string()),
        name: title,
        underlying,
        product_type,
        strike,
        maturity,
        bid_ask: format!("{bid} / {ask}"),
        last_price,
        last_trade_time: String::new(),
        detail_path: Some(path),
    })
}

fn preferred_boursorama_price(bid: &str, ask: &str) -> String {
    let ask_value = parse_decimal(ask).unwrap_or(0.0);
    let bid_value = parse_decimal(bid).unwrap_or(0.0);
    let price = if ask_value > 0.0 {
        ask
    } else if bid_value > 0.0 {
        bid
    } else {
        ask
    };
    format!("EUR {price}")
}

fn decode_data_rel(value: &str) -> Option<String> {
    let bytes = general_purpose::STANDARD.decode(value).ok()?;
    String::from_utf8(bytes).ok()
}

fn table_cells(row: &str) -> Vec<String> {
    row.split("<td")
        .skip(1)
        .filter_map(|part| {
            part.split_once("</td>")
                .map(|(cell, _)| strip_html(&format!("<td{cell}</td>")))
        })
        .map(|value| decode_entities(&value).split_whitespace().collect::<Vec<_>>().join(" "))
        .collect()
}

fn extract_attr(fragment: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let start = fragment.find(&needle)? + needle.len();
    let end = fragment[start..].find('"')? + start;
    Some(decode_entities(&fragment[start..end]))
}

fn extract_between<'a>(value: &'a str, start_marker: &str, end_marker: &str) -> Option<&'a str> {
    let start = value.find(start_marker)? + start_marker.len();
    let end = value[start..].find(end_marker)? + start;
    Some(&value[start..end])
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

    output
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
        .replace("&eacute;", "é")
        .replace("&agrave;", "à")
}

fn normalize_boursorama_maturity(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.eq_ignore_ascii_case("illimité") || trimmed.eq_ignore_ascii_case("illimite") {
        return "-".to_string();
    }

    let parts = trimmed.split('/').collect::<Vec<_>>();
    if parts.len() == 3 {
        let year = parts[2].trim().parse::<u32>().ok();
        if let Some(year) = year {
            return format!(
                "20{year:02}-{}-{}",
                parts[1].trim(),
                parts[0].trim()
            );
        }
    }

    trimmed.to_string()
}

fn parse_decimal(value: &str) -> Option<f64> {
    let normalized = value
        .trim()
        .replace(' ', "")
        .replace(',', ".");
    if normalized.is_empty() || normalized == "-" || normalized == "/" {
        return None;
    }
    normalized.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_boursorama_result_row() {
        let html = r#"
        <tr class="c-table__row" >
          <td><span data-rel="L2JvdXJzZS9wcm9kdWl0cy1kZS1ib3Vyc2UvY291cnMvd2FycmFudHMvM3JQREUwMDBGRDRUQUEx" title="Accéder à la fiche de HERMS/SGE WT 26">DE000FD4TAA1</span></td>
          <td></td>
          <td> Warrants Call </td>
          <td> HERMES IN… </td>
          <td data-sort-value="2488.93"> 2 488,930 </td>
          <td data-sort-value="20261218"> 18/12/26 </td>
          <td> 5,24% </td>
          <td data-sort-value="0"> 0,000 </td>
          <td data-sort-value="0.015"> 0,015 </td>
          <td> +13,33% </td>
          <td> 9,21 </td>
          <td> SOCIETE GENERALE </td>
        </tr>
        "#;

        let products = parse_boursorama_products(html);

        assert_eq!(products.len(), 1);
        assert_eq!(products[0].symbol, "3rPDE000FD4TAA1");
        assert_eq!(products[0].isin.as_deref(), Some("DE000FD4TAA1"));
        assert_eq!(products[0].product_type, "Warrants Call");
        assert_eq!(products[0].strike, "2 488,930");
        assert_eq!(products[0].maturity, "2026-12-18");
        assert_eq!(products[0].last_price, "EUR 0,015");
    }
}
