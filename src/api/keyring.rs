use anyhow::{anyhow, Result};
use reqwest::{Client, RequestBuilder, Response, StatusCode};
use std::collections::HashSet;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ApiKeyRing {
    keys: Vec<String>,
}

#[derive(Debug)]
pub struct KeyedResponse {
    pub key_index: usize,
    pub response: Response,
}

impl ApiKeyRing {
    pub fn from_env(name: &str) -> Self {
        std::env::var(name)
            .map(|value| Self::from_csv(&value))
            .unwrap_or_else(|_| Self { keys: Vec::new() })
    }

    pub fn from_csv(value: &str) -> Self {
        let mut seen = HashSet::new();
        let keys = value
            .split(',')
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .filter(|key| seen.insert((*key).to_string()))
            .map(str::to_string)
            .collect();

        Self { keys }
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub async fn send_with_rotation<F>(&self, client: &Client, build: F) -> Result<KeyedResponse>
    where
        F: Fn(&Client, &str) -> RequestBuilder,
    {
        if self.keys.is_empty() {
            return Err(anyhow!("aucune cle API configuree"));
        }

        let mut failures = Vec::new();
        for (index, key) in self.keys.iter().enumerate() {
            match build(client, key).send().await {
                Ok(response) if response.status().is_success() => {
                    return Ok(KeyedResponse {
                        key_index: index,
                        response,
                    });
                }
                Ok(response) if should_rotate_status(response.status()) => {
                    failures.push(format!("key#{} status {}", index + 1, response.status()));
                }
                Ok(response) => {
                    return Err(anyhow!(
                        "requete API refusee sans rotation: status {} avec key#{}",
                        response.status(),
                        index + 1
                    ));
                }
                Err(err) => {
                    failures.push(format!("key#{} network {}", index + 1, err));
                }
            }
        }

        Err(anyhow!(
            "toutes les cles API ont echoue ou sont rate-limitees: {}",
            failures.join("; ")
        ))
    }
}

pub fn should_rotate_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::UNAUTHORIZED
            | StatusCode::FORBIDDEN
            | StatusCode::TOO_MANY_REQUESTS
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    ) || status.is_server_error()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_comma_separated_keys_and_removes_duplicates() {
        let ring = ApiKeyRing::from_csv(" key-a, key-b ,,key-a,key-c ");

        assert_eq!(ring.len(), 3);
        assert_eq!(
            ring,
            ApiKeyRing {
                keys: vec![
                    "key-a".to_string(),
                    "key-b".to_string(),
                    "key-c".to_string()
                ]
            }
        );
    }

    #[test]
    fn classifies_statuses_that_should_try_next_key() {
        assert!(should_rotate_status(StatusCode::UNAUTHORIZED));
        assert!(should_rotate_status(StatusCode::FORBIDDEN));
        assert!(should_rotate_status(StatusCode::TOO_MANY_REQUESTS));
        assert!(should_rotate_status(StatusCode::INTERNAL_SERVER_ERROR));
        assert!(!should_rotate_status(StatusCode::BAD_REQUEST));
        assert!(!should_rotate_status(StatusCode::NOT_FOUND));
    }
}
