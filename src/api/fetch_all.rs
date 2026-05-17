use anyhow::{anyhow, Result};
use reqwest::Client;

use crate::models::warrant::WarrantSnapshot;
use crate::api::fetch::fetch_warrant;

/// Lance la récupération de tous les tickers en parallèle via tokio.
pub async fn fetch_all(client: &Client, tickers: &[String]) -> Vec<Result<WarrantSnapshot>> {
    let tasks: Vec<_> = tickers
        .iter()
        .map(|t| {
            let c = client.clone();
            let t = t.clone();
            tokio::spawn(async move { fetch_warrant(&c, &t).await })
        })
        .collect();

    let mut results = Vec::new();
    for task in tasks {
        match task.await {
            Ok(r)  => results.push(r),
            Err(e) => results.push(Err(anyhow!("Tâche tokio échouée: {e}"))),
        }
    }
    results
}
