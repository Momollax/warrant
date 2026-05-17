use anyhow::{Context, Result};
use reqwest::{Client, header};
use std::time::Duration;

/// Construit le client HTTP avec les headers requis par Yahoo Finance.
pub fn build_client() -> Result<Client> {
    let mut headers = header::HeaderMap::new();
    headers.insert(
        header::USER_AGENT,
        header::HeaderValue::from_static(
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/124.0 Safari/537.36",
        ),
    );
    headers.insert(
        header::ACCEPT,
        header::HeaderValue::from_static("application/json"),
    );
    headers.insert(
        header::ACCEPT_LANGUAGE,
        header::HeaderValue::from_static("fr-FR,fr;q=0.9,en;q=0.8"),
    );

    Client::builder()
        .default_headers(headers)
        .timeout(Duration::from_secs(15))
        .connect_timeout(Duration::from_secs(8))
        .build()
        .context("Impossible de créer le client HTTP")
}
