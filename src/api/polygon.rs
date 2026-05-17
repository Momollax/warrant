use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::Value;

use crate::api::keyring::ApiKeyRing;

const DEFAULT_POLYGON_BASE_URL: &str = "https://api.polygon.io";

#[derive(Debug, Clone)]
pub struct PolygonClient {
    base_url: String,
    keyring: ApiKeyRing,
}

impl PolygonClient {
    pub fn from_env() -> Self {
        Self {
            base_url: std::env::var("POLYGON_BASE_URL")
                .unwrap_or_else(|_| DEFAULT_POLYGON_BASE_URL.to_string()),
            keyring: ApiKeyRing::from_env("POLYGON_API_KEY"),
        }
    }

    pub fn from_key_csv(keys: &str) -> Self {
        Self {
            base_url: DEFAULT_POLYGON_BASE_URL.to_string(),
            keyring: ApiKeyRing::from_csv(keys),
        }
    }

    pub fn key_count(&self) -> usize {
        self.keyring.len()
    }

    pub async fn get_json(
        &self,
        client: &Client,
        path: &str,
        params: &[(&str, &str)],
    ) -> Result<Value> {
        let path = path.trim_start_matches('/');
        let url = format!("{}/{}", self.base_url.trim_end_matches('/'), path);
        let keyed_response = self
            .keyring
            .send_with_rotation(client, |client, api_key| {
                client.get(&url).query(&[("apiKey", api_key)]).query(params)
            })
            .await
            .with_context(|| format!("Polygon request failed for /{path}"))?;

        keyed_response
            .response
            .json::<Value>()
            .await
            .with_context(|| {
                format!(
                    "Polygon returned non-JSON payload for /{path} with key#{}",
                    keyed_response.key_index + 1
                )
            })
    }

    pub async fn previous_aggregate(&self, client: &Client, ticker: &str) -> Result<Value> {
        let path = format!("v2/aggs/ticker/{ticker}/prev");
        self.get_json(client, &path, &[("adjusted", "true")]).await
    }

    #[allow(dead_code)]
    pub async fn dividends(&self, client: &Client, ticker: &str) -> Result<Value> {
        self.get_json(client, "v3/reference/dividends", &[("ticker", ticker)])
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polygon_client_accepts_multiple_comma_separated_keys() {
        let client = PolygonClient::from_key_csv("key-a, key-b, key-a");

        assert_eq!(client.key_count(), 2);
    }
}
