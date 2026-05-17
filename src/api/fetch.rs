use anyhow::{anyhow, Context, Result};
use reqwest::Client;

use crate::models::yahoo::YahooResponse;
use crate::models::warrant::WarrantSnapshot;
use crate::parser::parse_snapshot::parse_snapshot;

/// Récupère les données d'un ticker via l'API chart de Yahoo Finance.
/// interval=1m, range=1d → dernière bougie + métadonnées temps réel.
pub async fn fetch_warrant(client: &Client, ticker: &str) -> Result<WarrantSnapshot> {
    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{ticker}\
         ?interval=1m&range=1d&includePrePost=false",
        ticker = ticker
    );

    let response = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("Erreur HTTP pour {ticker}"))?;

    let status = response.status();
    if status == 404 {
        return Err(anyhow!(
            "Ticker '{ticker}' introuvable (404) — warrant expiré/délisté ou ticker invalide.\n\
             → Vérifiez sur https://finance.yahoo.com que le ticker existe bien."
        ));
    }
    if !status.is_success() {
        return Err(anyhow!(
            "Yahoo Finance a répondu {} pour {ticker}",
            status
        ));
    }

    let raw: YahooResponse = response
        .json()
        .await
        .with_context(|| format!("Impossible de parser le JSON pour {ticker}"))?;

    if let Some(err) = &raw.chart.error {
        return Err(anyhow!("Erreur Yahoo pour {ticker}: {err}"));
    }

    let result = raw
        .chart
        .result
        .and_then(|mut v| v.pop())
        .ok_or_else(|| anyhow!("Aucun résultat pour {ticker}"))?;

    parse_snapshot(result)
}
