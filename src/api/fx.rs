use reqwest::Client;

use crate::api::fetch::fetch_warrant;
use crate::models::fx::FxRate;

pub async fn fetch_fx_rate(client: &Client, from: &str, to: &str) -> Option<FxRate> {
    if from.eq_ignore_ascii_case(to) {
        return Some(FxRate {
            from: from.to_ascii_uppercase(),
            to: to.to_ascii_uppercase(),
            rate: 1.0,
            source_ticker: "IDENTITY".to_string(),
        });
    }

    if is_eur_usd_pair(from, to) {
        return fetch_eur_usd_rate(client, from, to).await;
    }

    fetch_direct_or_inverse_rate(client, from, to).await
}

async fn fetch_eur_usd_rate(client: &Client, from: &str, to: &str) -> Option<FxRate> {
    let snapshot = fetch_warrant(client, "EURUSD=X").await.ok()?;
    if !snapshot.price.is_finite() || snapshot.price <= 0.0 {
        return None;
    }

    let rate = if from.eq_ignore_ascii_case("EUR") && to.eq_ignore_ascii_case("USD") {
        snapshot.price
    } else {
        1.0 / snapshot.price
    };

    Some(FxRate {
        from: from.to_ascii_uppercase(),
        to: to.to_ascii_uppercase(),
        rate,
        source_ticker: "EURUSD=X".to_string(),
    })
}

async fn fetch_direct_or_inverse_rate(client: &Client, from: &str, to: &str) -> Option<FxRate> {
    let direct_ticker = format!("{}{}=X", from.to_ascii_uppercase(), to.to_ascii_uppercase());
    if let Ok(snapshot) = fetch_warrant(client, &direct_ticker).await {
        if snapshot.price.is_finite() && snapshot.price > 0.0 {
            return Some(FxRate {
                from: from.to_ascii_uppercase(),
                to: to.to_ascii_uppercase(),
                rate: snapshot.price,
                source_ticker: direct_ticker,
            });
        }
    }

    let inverse_ticker = format!("{}{}=X", to.to_ascii_uppercase(), from.to_ascii_uppercase());
    if let Ok(snapshot) = fetch_warrant(client, &inverse_ticker).await {
        if snapshot.price.is_finite() && snapshot.price > 0.0 {
            return Some(FxRate {
                from: from.to_ascii_uppercase(),
                to: to.to_ascii_uppercase(),
                rate: 1.0 / snapshot.price,
                source_ticker: inverse_ticker,
            });
        }
    }

    None
}

fn is_eur_usd_pair(from: &str, to: &str) -> bool {
    (from.eq_ignore_ascii_case("EUR") && to.eq_ignore_ascii_case("USD"))
        || (from.eq_ignore_ascii_case("USD") && to.eq_ignore_ascii_case("EUR"))
}
