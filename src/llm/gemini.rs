use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::api::keyring::ApiKeyRing;
use crate::decision::scenario::{ScenarioCandidate, ScenarioConfig};
use crate::models::warrant::WarrantSnapshot;

const DEFAULT_GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com";
const DEFAULT_GEMINI_MODEL: &str = "gemini-3.1-pro-preview";
const DEFAULT_CACHE_DIR: &str = "data/cache/llm";

#[derive(Debug, Clone)]
pub struct GeminiConfig {
    pub enabled: bool,
    pub base_url: String,
    pub model: String,
    pub cache_enabled: bool,
    pub cache_dir: PathBuf,
    pub keyring: ApiKeyRing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmScenarioReview {
    pub verdict: String,
    pub confidence: u8,
    pub summary: String,
    pub main_risks: Vec<String>,
    pub data_issues: Vec<String>,
    pub trade_plan_review: String,
    pub questions_before_entry: Vec<String>,
}

impl GeminiConfig {
    pub fn from_env() -> Self {
        let keyring = std::env::var("GEMINI_API_KEY")
            .or_else(|_| std::env::var("LLM_API_KEY"))
            .map(|value| ApiKeyRing::from_csv(&value))
            .unwrap_or_else(|_| ApiKeyRing::from_csv(""));
        let enabled = env_flag("LLM_ENABLE").unwrap_or_else(|| keyring.len() > 0);

        Self {
            enabled,
            base_url: std::env::var("GEMINI_BASE_URL")
                .or_else(|_| std::env::var("LLM_BASE_URL"))
                .unwrap_or_else(|_| DEFAULT_GEMINI_BASE_URL.to_string()),
            model: std::env::var("GEMINI_MODEL")
                .or_else(|_| std::env::var("LLM_MODEL"))
                .unwrap_or_else(|_| DEFAULT_GEMINI_MODEL.to_string()),
            cache_enabled: env_flag("LLM_CACHE").unwrap_or(true),
            cache_dir: PathBuf::from(
                std::env::var("LLM_CACHE_DIR").unwrap_or_else(|_| DEFAULT_CACHE_DIR.to_string()),
            ),
            keyring,
        }
    }

    pub fn disabled_reason(&self) -> Option<String> {
        if !self.enabled {
            return Some("LLM desactive: mets LLM_ENABLE=1 dans .env.".to_string());
        }
        if self.keyring.len() == 0 {
            return Some("Aucune cle Gemini: mets GEMINI_API_KEY=ta_cle dans .env.".to_string());
        }
        None
    }
}

pub async fn analyze_scenario_candidate(
    client: &Client,
    config: &GeminiConfig,
    underlying: &WarrantSnapshot,
    scenario: &ScenarioConfig,
    candidate: &ScenarioCandidate,
) -> Result<LlmScenarioReview> {
    if let Some(reason) = config.disabled_reason() {
        return Err(anyhow!(reason));
    }

    let prompt = build_scenario_prompt(underlying, scenario, candidate);
    let cache_path = cache_path(&config.cache_dir, &config.model, &prompt);
    if config.cache_enabled {
        if let Ok(cached) = fs::read_to_string(&cache_path) {
            let review = parse_review(&cached).context("cache LLM Gemini invalide")?;
            return Ok(sanitize_review(review));
        }
    }

    let payload = json!({
        "contents": [{
            "role": "user",
            "parts": [{ "text": prompt }]
        }],
        "generationConfig": {
            "temperature": 0.15,
            "topP": 0.9,
            "maxOutputTokens": 4096,
            "responseMimeType": "application/json",
            "responseSchema": {
                "type": "OBJECT",
                "properties": {
                    "verdict": { "type": "STRING" },
                    "confidence": { "type": "INTEGER" },
                    "summary": { "type": "STRING" },
                    "main_risks": { "type": "ARRAY", "items": { "type": "STRING" } },
                    "data_issues": { "type": "ARRAY", "items": { "type": "STRING" } },
                    "trade_plan_review": { "type": "STRING" },
                    "questions_before_entry": { "type": "ARRAY", "items": { "type": "STRING" } }
                },
                "required": [
                    "verdict",
                    "confidence",
                    "summary",
                    "main_risks",
                    "data_issues",
                    "trade_plan_review",
                    "questions_before_entry"
                ]
            }
        }
    });

    let endpoint = format!(
        "{}/v1beta/models/{}:generateContent",
        config.base_url.trim_end_matches('/'),
        config.model
    );
    let response = config
        .keyring
        .send_with_rotation(client, |client, key| {
            client.post(&endpoint).query(&[("key", key)]).json(&payload)
        })
        .await?
        .response
        .json::<Value>()
        .await
        .context("reponse JSON Gemini invalide")?;
    let text = extract_gemini_text(&response)?;
    let review = sanitize_review(parse_review(&text)?);

    if config.cache_enabled {
        let _ = fs::create_dir_all(&config.cache_dir);
        let _ = fs::write(cache_path, serde_json::to_string_pretty(&review)?);
    }

    Ok(review)
}

pub fn build_scenario_prompt(
    underlying: &WarrantSnapshot,
    scenario: &ScenarioConfig,
    candidate: &ScenarioCandidate,
) -> String {
    let context = json!({
        "role": "Tu es un auditeur de signaux sur warrants. Tu ne donnes pas d'ordre financier; tu controles la coherence du signal, les risques, les donnees manquantes et les points a verifier avant execution.",
        "underlying": {
            "ticker": underlying.ticker,
            "price": underlying.price,
            "currency": underlying.currency,
            "change_pct": underlying.change_pct
        },
        "scenario": {
            "side": scenario.side,
            "target_price": scenario.target_price,
            "target_date": scenario.target_date.to_string(),
            "min_maturity": scenario.min_maturity.map(|date| date.to_string()),
            "max_maturity": scenario.max_maturity.map(|date| date.to_string()),
            "risk_free_rate": scenario.risk_free_rate,
            "dividend_yield": scenario.dividend_yield,
            "fallback_volatility": scenario.fallback_volatility
        },
        "candidate": {
            "decision": candidate.decision,
            "score": candidate.score,
            "symbol": candidate.symbol,
            "side": candidate.side,
            "product_type": candidate.product_type,
            "url": candidate.url,
            "strike": candidate.strike,
            "maturity": candidate.maturity.to_string(),
            "entry_price": candidate.entry_price,
            "projected_price_at_target": candidate.projected_price,
            "net_return_pct": candidate.net_return_pct,
            "gross_return_pct": candidate.gross_return_pct,
            "fee_drag_pct": candidate.fee_drag_pct,
            "breakeven_underlying": candidate.breakeven_underlying,
            "breakeven_distance_pct": candidate.breakeven_distance_pct,
            "implied_volatility": candidate.implied_volatility,
            "volatility_used": candidate.volatility_used,
            "theta_horizon_pct": candidate.theta_horizon_pct,
            "spread_pct": candidate.spread_pct,
            "probability_target_pct": candidate.probability_target_pct,
            "probability_breakeven_pct": candidate.probability_breakeven_pct,
            "expected_value_pct": candidate.expected_value_pct,
            "sharpe_like": candidate.sharpe_like,
            "kelly_fraction_pct": candidate.kelly_fraction_pct,
            "delta": candidate.delta,
            "gamma": candidate.gamma,
            "vega_per_vol_point": candidate.vega_per_vol_point,
            "theta_per_day": candidate.theta_per_day,
            "rho_per_rate_point": candidate.rho_per_rate_point,
            "data_quality_score": candidate.data_quality_score,
            "flow_score_informational_only": candidate.liquidity_score,
            "flow_score_note": "Flux/volume entre intervenants observe. Pour ces produits, il est informatif uniquement: l'execution depend surtout du market maker, du bid/ask, du spread et du statut de cotation. Ce champ ne doit pas servir a conclure sur l'entree ou la sortie.",
            "parity": candidate.parity,
            "reasons": candidate.reasons
        }
    });

    format!(
        "Analyse ce warrant en francais a partir du JSON ci-dessous.\n\
         Reponds uniquement en JSON valide avec ce schema exact:\n\
         {{\"verdict\":\"BUY|WATCH|AVOID\",\"confidence\":0-100,\"summary\":\"...\",\
         \"main_risks\":[\"...\"],\"data_issues\":[\"...\"],\
         \"trade_plan_review\":\"...\",\"questions_before_entry\":[\"...\"]}}\n\
         Regle importante: ne considere jamais flow_score_informational_only comme une raison de BUY/WATCH/AVOID. N'utilise pas ce score pour conclure a un probleme d'execution ou de carnet; pour ces produits, le flux/volume est informatif seulement car l'investisseur traite surtout contre le market maker. Les vrais criteres d'execution sont bid/ask executable, spread, taille affichee si disponible, statut de cotation et fraicheur des donnees.\n\
         Le verdict doit rester prudent: si donnees manquantes, spread large, EV/Kelly faibles ou prix non executable, favorise WATCH/AVOID.\n\
         JSON contexte:\n{}",
        serde_json::to_string_pretty(&context).unwrap_or_else(|_| "{}".to_string())
    )
}

pub fn parse_review(raw: &str) -> Result<LlmScenarioReview> {
    let cleaned = strip_markdown_fence(raw);
    if let Ok(review) = serde_json::from_str::<LlmScenarioReview>(&cleaned) {
        return Ok(review);
    }
    if let Some(json_object) = extract_first_json_object(&cleaned) {
        if let Ok(review) = serde_json::from_str::<LlmScenarioReview>(&json_object) {
            return Ok(review);
        }
    }

    Ok(LlmScenarioReview {
        verdict: "WATCH".to_string(),
        confidence: 0,
        summary: "Gemini a repondu, mais pas dans le JSON strict attendu. L'analyse brute est affichee dans les problemes de donnees.".to_string(),
        main_risks: vec!["Reponse LLM non structuree ou tronquee; ne pas utiliser cet avis comme signal.".to_string()],
        data_issues: vec![format!("Extrait brut Gemini: {}", compact_excerpt(&cleaned, 900))],
        trade_plan_review: "Relance l'analyse avec r. Si le probleme persiste, utilise gemini-2.5-pro ou gemini-2.5-flash, ou reduis le contexte envoye.".to_string(),
        questions_before_entry: vec![
            "La reponse Gemini est-elle complete apres relance ?".to_string(),
            "Le verdict quantitatif reste-t-il coherent sans l'avis LLM ?".to_string(),
        ],
    })
}

fn sanitize_review(mut review: LlmScenarioReview) -> LlmScenarioReview {
    review.summary = sanitize_liquidity_language(&review.summary);
    review.trade_plan_review = sanitize_liquidity_language(&review.trade_plan_review);
    review.main_risks = sanitize_review_list(review.main_risks);
    review.data_issues = sanitize_review_list(review.data_issues);
    review.questions_before_entry = sanitize_review_list(review.questions_before_entry);
    review
}

fn sanitize_review_list(items: Vec<String>) -> Vec<String> {
    let mut sanitized = Vec::with_capacity(items.len());
    for item in items {
        let item = sanitize_liquidity_language(&item);
        if !sanitized.iter().any(|existing| existing == &item) {
            sanitized.push(item);
        }
    }
    sanitized
}

fn sanitize_liquidity_language(text: &str) -> String {
    if references_liquidity_as_risk(text) {
        return "Execution a verifier via bid/ask, spread, tailles affichees et statut de cotation.".to_string();
    }
    text.to_string()
}

fn references_liquidity_as_risk(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("liquidit") || lower.contains("liquidity")
}

fn strip_markdown_fence(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with("```") {
        let without_open = trimmed
            .strip_prefix("```json")
            .or_else(|| trimmed.strip_prefix("```JSON"))
            .or_else(|| trimmed.strip_prefix("```"))
            .unwrap_or(trimmed)
            .trim();
        return without_open
            .strip_suffix("```")
            .unwrap_or(without_open)
            .trim()
            .to_string();
    }
    trimmed.to_string()
}

fn extract_first_json_object(raw: &str) -> Option<String> {
    let mut start = None;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (index, ch) in raw.char_indices() {
        if start.is_none() {
            if ch == '{' {
                start = Some(index);
                depth = 1;
            }
            continue;
        }

        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && in_string {
            escaped = true;
            continue;
        }
        if ch == '"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }

        match ch {
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let start = start?;
                    return Some(raw[start..=index].to_string());
                }
            }
            _ => {}
        }
    }

    None
}

fn compact_excerpt(raw: &str, max_chars: usize) -> String {
    let compact = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max_chars {
        return compact;
    }
    compact.chars().take(max_chars).collect::<String>() + "..."
}

fn extract_gemini_text(value: &Value) -> Result<String> {
    value
        .get("candidates")
        .and_then(|candidates| candidates.as_array())
        .and_then(|candidates| candidates.first())
        .and_then(|candidate| candidate.get("content"))
        .and_then(|content| content.get("parts"))
        .and_then(|parts| parts.as_array())
        .and_then(|parts| parts.first())
        .and_then(|part| part.get("text"))
        .and_then(|text| text.as_str())
        .map(str::to_string)
        .ok_or_else(|| anyhow!("Gemini n'a pas renvoye de texte exploitable"))
}

fn cache_path(cache_dir: &Path, model: &str, prompt: &str) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    model.hash(&mut hasher);
    prompt.hash(&mut hasher);
    cache_dir.join(format!("{:016x}.json", hasher.finish()))
}

fn env_flag(name: &str) -> Option<bool> {
    std::env::var(name).ok().map(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn parses_json_review_even_when_wrapped_in_markdown() {
        let review = parse_review(
            r#"```json
{"verdict":"WATCH","confidence":72,"summary":"Signal coherent mais liquidite faible.","main_risks":["liquidite"],"data_issues":[],"trade_plan_review":"Plan prudent.","questions_before_entry":["Carnet actif ?"]}
```"#,
        )
        .unwrap();

        assert_eq!(review.verdict, "WATCH");
        assert_eq!(review.confidence, 72);
        assert_eq!(review.main_risks, vec!["liquidite"]);
    }

    #[test]
    fn parses_json_review_inside_extra_text() {
        let review = parse_review(
            r#"Voici l'analyse:
{"verdict":"AVOID","confidence":61,"summary":"Trop fragile.","main_risks":["spread"],"data_issues":["bid ask"],"trade_plan_review":"Attendre.","questions_before_entry":["Carnet ?"]}
Fin."#,
        )
        .unwrap();

        assert_eq!(review.verdict, "AVOID");
        assert_eq!(review.data_issues, vec!["bid ask"]);
    }

    #[test]
    fn partial_or_invalid_json_becomes_displayable_review() {
        let review = parse_review(
            r#"{"verdict":"WATCH","confidence":85,"summary":"Analyse incomplete","#,
        )
        .unwrap();

        assert_eq!(review.verdict, "WATCH");
        assert_eq!(review.confidence, 0);
        assert!(review.data_issues[0].contains("Extrait brut Gemini"));
    }

    #[test]
    fn prompt_contains_candidate_and_prudence_rules() {
        let prompt = build_scenario_prompt(&underlying(), &scenario(), &candidate());

        assert!(prompt.contains("\"symbol\": \"D12QS\""));
        assert!(prompt.contains("\"target_price\": 330.0"));
        assert!(prompt.contains("favorise WATCH/AVOID"));
        assert!(prompt.contains("flow_score_informational_only"));
        assert!(prompt.contains("ne considere jamais flow_score_informational_only"));
        assert!(!prompt.contains("\"liquidity_score\""));
        assert!(!prompt.contains("liquidite faible"));
    }

    #[test]
    fn sanitizes_liquidity_language_from_llm_review() {
        let review = LlmScenarioReview {
            verdict: "WATCH".to_string(),
            confidence: 80,
            summary: "Liquidite faible sur ce produit.".to_string(),
            main_risks: vec![
                "Liquidite insuffisante".to_string(),
                "Spread large".to_string(),
                "liquidity risk".to_string(),
            ],
            data_issues: vec!["Aucun probleme".to_string()],
            trade_plan_review: "Plan coherent, mais liquidite faible.".to_string(),
            questions_before_entry: vec![
                "La liquidite permet-elle de sortir ?".to_string(),
                "Le spread est-il stable ?".to_string(),
            ],
        };

        let sanitized = sanitize_review(review);

        assert!(!format!("{sanitized:?}").to_lowercase().contains("liquidit"));
        assert!(sanitized.main_risks.iter().any(|risk| risk == "Spread large"));
        assert!(sanitized
            .main_risks
            .iter()
            .any(|risk| risk.contains("Execution a verifier")));
    }

    fn underlying() -> WarrantSnapshot {
        WarrantSnapshot {
            ticker: "AAPL".to_string(),
            name: "Apple".to_string(),
            price: 297.29,
            currency: "USD".to_string(),
            prev_close: 300.23,
            change_pct: -0.98,
            volume: 0,
            high_52w: 0.0,
            low_52w: 0.0,
            last_candle: None,
            fetched_at: chrono::Utc::now(),
        }
    }

    fn scenario() -> ScenarioConfig {
        ScenarioConfig {
            target_price: 330.0,
            target_date: NaiveDate::from_ymd_opt(2026, 10, 31).unwrap(),
            min_maturity: Some(NaiveDate::from_ymd_opt(2027, 1, 1).unwrap()),
            max_maturity: Some(NaiveDate::from_ymd_opt(2027, 3, 31).unwrap()),
            side: "call".to_string(),
            risk_free_rate: 0.045,
            dividend_yield: 0.005,
            fallback_volatility: 0.28,
        }
    }

    fn candidate() -> ScenarioCandidate {
        ScenarioCandidate {
            decision: "WATCH",
            score: 53.0,
            symbol: "D12QS".to_string(),
            side: "call".to_string(),
            product_type: "Warrant Call".to_string(),
            url: "https://example.test".to_string(),
            strike: 280.0,
            maturity: NaiveDate::from_ymd_opt(2027, 3, 19).unwrap(),
            entry_price: 3.8,
            projected_price: 5.01,
            net_return_pct: 31.36,
            gross_return_pct: 31.8,
            fee_drag_pct: 0.38,
            breakeven_underlying: Some(313.26),
            breakeven_distance_pct: Some(5.37),
            implied_volatility: Some(0.2837),
            volatility_used: 0.2837,
            theta_horizon_pct: Some(0.0),
            spread_pct: Some(0.53),
            probability_target_pct: Some(29.25),
            probability_breakeven_pct: Some(39.2),
            target_zscore: Some(0.546),
            breakeven_zscore: Some(0.274),
            expected_value_pct: Some(1.68),
            sharpe_like: Some(-1.03),
            kelly_fraction_pct: Some(0.0),
            delta: Some(0.0588),
            gamma: Some(0.000393),
            vega_per_vol_point: Some(0.082),
            theta_per_day: Some(-0.005),
            rho_per_rate_point: Some(0.114),
            d1: Some(0.49),
            d2: Some(0.23),
            data_quality_score: 100.0,
            liquidity_score: 45.0,
            parity: 10.0,
            reasons: vec![],
        }
    }
}
