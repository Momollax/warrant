use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::api::keyring::ApiKeyRing;
use crate::decision::scenario::{ScenarioCandidate, ScenarioConfig};
use crate::models::warrant::WarrantSnapshot;

const DEFAULT_GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com";
const DEFAULT_GEMINI_MODEL: &str = "gemini-3-pro-preview";
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
    #[serde(default)]
    pub decision_drivers: Vec<String>,
    #[serde(default)]
    pub red_flags: Vec<String>,
    pub main_risks: Vec<String>,
    #[serde(default)]
    pub execution_checks: Vec<String>,
    pub data_issues: Vec<String>,
    pub trade_plan_review: String,
    #[serde(default)]
    pub invalidation_conditions: Vec<String>,
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

    let system_instruction = gemini_system_instruction();
    let prompt = build_scenario_prompt(underlying, scenario, candidate);
    let cache_material = format!("{system_instruction}\n\n{prompt}");
    let cache_path = cache_path(&config.cache_dir, &config.model, &cache_material);
    if config.cache_enabled {
        if let Ok(cached) = fs::read_to_string(&cache_path) {
            let review = parse_review(&cached).context("cache LLM Gemini invalide")?;
            return Ok(sanitize_review(review));
        }
    }

    let payload = json!({
        "systemInstruction": {
            "parts": [{ "text": system_instruction }]
        },
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
                    "decision_drivers": { "type": "ARRAY", "items": { "type": "STRING" } },
                    "red_flags": { "type": "ARRAY", "items": { "type": "STRING" } },
                    "main_risks": { "type": "ARRAY", "items": { "type": "STRING" } },
                    "execution_checks": { "type": "ARRAY", "items": { "type": "STRING" } },
                    "data_issues": { "type": "ARRAY", "items": { "type": "STRING" } },
                    "trade_plan_review": { "type": "STRING" },
                    "invalidation_conditions": { "type": "ARRAY", "items": { "type": "STRING" } },
                    "questions_before_entry": { "type": "ARRAY", "items": { "type": "STRING" } }
                },
                "required": [
                    "verdict",
                    "confidence",
                    "summary",
                    "decision_drivers",
                    "red_flags",
                    "main_risks",
                    "execution_checks",
                    "data_issues",
                    "trade_plan_review",
                    "invalidation_conditions",
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
        "audit_version": 2,
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
            "fallback_volatility": scenario.fallback_volatility,
            "volatility_shock_points": scenario.volatility_shock_points,
            "real_world_drift": scenario.real_world_drift,
            "fx_target_rate": scenario.fx_target_rate,
            "fx_stress_pct": scenario.fx_stress_pct,
            "vol_spot_slope_points_per_pct": scenario.vol_spot_slope_points_per_pct,
            "exit_spread_multiplier": scenario.exit_spread_multiplier,
            "exit_spread_delta_penalty": scenario.exit_spread_delta_penalty,
            "stale_pricing_guard": scenario.stale_pricing_guard,
            "dividends": scenario.dividends.iter().map(|dividend| json!({
                "ex_date": dividend.ex_date.to_string(),
                "amount": dividend.amount
            })).collect::<Vec<_>>()
        },
        "candidate": {
            "decision": candidate.decision,
            "score": candidate.score,
            "symbol": candidate.symbol,
            "side": candidate.side,
            "product_family": candidate.product_family,
            "pricing_model": candidate.pricing_model,
            "product_type": candidate.product_type,
            "url": candidate.url,
            "strike": candidate.strike,
            "maturity": candidate.maturity_label,
            "entry_price": candidate.entry_price,
            "projected_price_at_target": candidate.projected_price,
            "projected_bid_price_at_target": candidate.projected_bid_price,
            "stressed_projected_price_at_target": candidate.stressed_projected_price,
            "fx_stressed_projected_price_at_target": candidate.fx_stressed_projected_price,
            "net_return_pct": candidate.net_return_pct,
            "stressed_net_return_pct": candidate.stressed_net_return_pct,
            "fx_stressed_net_return_pct": candidate.fx_stressed_net_return_pct,
            "gross_return_pct": candidate.gross_return_pct,
            "fee_drag_pct": candidate.fee_drag_pct,
            "breakeven_underlying": candidate.breakeven_underlying,
            "breakeven_distance_pct": candidate.breakeven_distance_pct,
            "implied_volatility": candidate.implied_volatility,
            "entry_volatility_used": candidate.volatility_used,
            "exit_volatility_used": candidate.exit_volatility,
            "dynamic_volatility_shift_points": candidate.dynamic_volatility_shift_points,
            "volatility_shock_points": candidate.volatility_shock_points,
            "theta_horizon_pct": candidate.theta_horizon_pct,
            "spread_pct": candidate.spread_pct,
            "first_touch_probability_target_pct_real_world": candidate.probability_target_pct,
            "first_touch_probability_breakeven_pct_real_world": candidate.probability_breakeven_pct,
            "terminal_probability_target_pct_risk_neutral": candidate.terminal_probability_target_pct,
            "terminal_probability_breakeven_pct_risk_neutral": candidate.terminal_probability_breakeven_pct,
            "expected_value_pct": candidate.expected_value_pct,
            "sharpe_like": candidate.sharpe_like,
            "kelly_fraction_pct_tp_sl_bounded": candidate.kelly_fraction_pct,
            "delta": candidate.delta,
            "gamma": candidate.gamma,
            "target_delta": candidate.target_delta,
            "target_gamma": candidate.target_gamma,
            "vega_per_vol_point": candidate.vega_per_vol_point,
            "target_vega_per_vol_point": candidate.target_vega_per_vol_point,
            "theta_per_day": candidate.theta_per_day,
            "target_theta_per_day": candidate.target_theta_per_day,
            "rho_per_rate_point": candidate.rho_per_rate_point,
            "data_quality_score": candidate.data_quality_score,
            "flow_score_informational_only": candidate.liquidity_score,
            "flow_score_note": "Flux/volume entre intervenants observe. Pour ces produits, il est informatif uniquement: l'execution depend surtout du market maker, du bid/ask, du spread et du statut de cotation. Ce champ ne doit pas servir a conclure sur l'entree ou la sortie.",
            "parity": candidate.parity,
            "fx_entry_rate": candidate.fx_rate,
            "fx_exit_rate": candidate.fx_exit_rate,
            "fx_stressed_exit_rate": candidate.fx_stressed_exit_rate,
            "effective_exit_spread_multiplier": candidate.effective_exit_spread_multiplier,
            "discrete_dividend_pv": candidate.discrete_dividend_pv,
            "projected_payoff_reference": candidate.projected_reference,
            "projected_barrier": candidate.projected_barrier,
            "financing_drag_pct": candidate.financing_drag_pct,
            "barrier_touch_probability_pct": candidate.barrier_touch_probability_pct,
            "monte_carlo": {
                "target_first_pct": candidate.monte_carlo_target_first_pct,
                "stop_first_pct": candidate.monte_carlo_stop_first_pct,
                "knock_out_pct": candidate.monte_carlo_ko_pct,
                "expected_return_pct": candidate.monte_carlo_expected_return_pct,
                "p05_return_pct": candidate.monte_carlo_p05_return_pct,
                "p50_return_pct": candidate.monte_carlo_p50_return_pct,
                "p95_return_pct": candidate.monte_carlo_p95_return_pct
            },
            "linear_model_note": if candidate.pricing_model == "warrant_intrinsic" {
                Value::Null
            } else {
                json!("Produit lineaire: IV/theta/vega Black-Scholes non utilises; projection basee sur reference projetee si SCENARIO_LINEAR_FINANCING_RATE_PCT est renseigne, sinon reference actuelle. Le risque barriere/KO et le Monte Carlo doivent primer sur l'IV.")
            },
            "reasons": candidate.reasons,
            "warnings": candidate.warnings
        }
    });

    format!(
        "Analyse ce warrant en francais a partir du JSON ci-dessous.\n\
         Reponds uniquement en JSON valide avec ce schema exact:\n\
         {{\"verdict\":\"BUY|WATCH|AVOID\",\"confidence\":0-100,\"summary\":\"...\",\
         \"decision_drivers\":[\"...\"],\"red_flags\":[\"...\"],\"main_risks\":[\"...\"],\
         \"execution_checks\":[\"...\"],\"data_issues\":[\"...\"],\
         \"trade_plan_review\":\"...\",\"invalidation_conditions\":[\"...\"],\
         \"questions_before_entry\":[\"...\"]}}\n\
         JSON contexte:\n{}",
        serde_json::to_string_pretty(&context).unwrap_or_else(|_| "{}".to_string())
    )
}

fn gemini_system_instruction() -> &'static str {
    "Tu es un auditeur de signaux sur warrants. Tu ne donnes pas d'ordre financier; tu controles la coherence du signal, les risques, les donnees manquantes et les points a verifier avant execution.\n\
     Methode obligatoire:\n\
     1. Respecte le verdict quantitatif sauf incoherence manifeste. Si tu contredis le moteur, explique pourquoi dans red_flags.\n\
     2. Base le verdict sur: rendement net, P/L EUR, stress IV, stress FX, breakeven, first-touch, KO%, Monte Carlo target/stop/KO, spread de sortie, bid/ask executable, stale pricing, dividendes discrets, greeks projetes et data quality.\n\
     3. Ne considere jamais flow_score_informational_only comme une raison de BUY/WATCH/AVOID. Le flux/volume est informatif seulement; les vrais criteres d'execution sont bid/ask, spread, tailles, statut et fraicheur des donnees.\n\
     4. Distingue probabilite first-touch monde reel et probabilite terminale risque-neutre. Ne presente pas le Kelly comme une certitude.\n\
     5. Sois concis: max 2 phrases pour summary, 3 a 5 items par liste, pas de conseil financier direct.\n\
     6. Favorise WATCH/AVOID si donnees manquantes, spread large, EV/Kelly faibles ou negatifs, risque FX/IV fort, stale pricing, prix non executable ou target avant breakeven.\n\
     7. Pour les produits lineaires open-end, ne traite pas IV/theta/vega comme des criteres Black-Scholes. Si linear_model_note indique que le financement futur n'est pas projete sur un horizon long, cite ce point comme limite du signal plutot qu'un probleme de liquidite."
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
        decision_drivers: vec!["Reponse non structuree: utiliser uniquement le moteur quantitatif.".to_string()],
        red_flags: vec!["Reponse LLM non structuree ou tronquee.".to_string()],
        main_risks: vec!["Reponse LLM non structuree ou tronquee; ne pas utiliser cet avis comme signal.".to_string()],
        execution_checks: vec!["Relancer Gemini ou verifier le bid/ask manuellement.".to_string()],
        data_issues: vec![format!("Extrait brut Gemini: {}", compact_excerpt(&cleaned, 900))],
        trade_plan_review: "Relance l'analyse avec r. Si le probleme persiste, utilise gemini-2.5-pro ou gemini-2.5-flash, ou reduis le contexte envoye.".to_string(),
        invalidation_conditions: vec!["Avis LLM invalide tant que la reponse n'est pas structuree.".to_string()],
        questions_before_entry: vec![
            "La reponse Gemini est-elle complete apres relance ?".to_string(),
            "Le verdict quantitatif reste-t-il coherent sans l'avis LLM ?".to_string(),
        ],
    })
}

fn sanitize_review(mut review: LlmScenarioReview) -> LlmScenarioReview {
    review.summary = sanitize_liquidity_language(&review.summary);
    review.trade_plan_review = sanitize_liquidity_language(&review.trade_plan_review);
    review.decision_drivers = sanitize_review_list(review.decision_drivers);
    review.red_flags = sanitize_review_list(review.red_flags);
    review.main_risks = sanitize_review_list(review.main_risks);
    review.execution_checks = sanitize_review_list(review.execution_checks);
    review.data_issues = sanitize_review_list(review.data_issues);
    review.invalidation_conditions = sanitize_review_list(review.invalidation_conditions);
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
    if let Some(block_reason) = value
        .get("promptFeedback")
        .and_then(|feedback| feedback.get("blockReason"))
    {
        return Err(anyhow!(
            "Gemini a bloque la requete (safety/filter): {}",
            block_reason
        ));
    }

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
        .ok_or_else(|| {
            anyhow!(
                "Gemini n'a pas renvoye de texte exploitable. Payload: {}",
                compact_excerpt(&value.to_string(), 1200)
            )
        })
}

fn cache_path(cache_dir: &Path, model: &str, prompt: &str) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(model.as_bytes());
    hasher.update(b"\0");
    hasher.update(prompt.as_bytes());
    cache_dir.join(format!("{:x}.json", hasher.finalize()))
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
    fn prompt_contains_candidate_and_system_rules_are_separated() {
        let prompt = build_scenario_prompt(&underlying(), &scenario(), &candidate());
        let system = gemini_system_instruction();

        assert!(prompt.contains("\"symbol\": \"D12QS\""));
        assert!(prompt.contains("\"target_price\": 330.0"));
        assert!(prompt.contains("flow_score_informational_only"));
        assert!(system.to_lowercase().contains("favorise watch/avoid"));
        assert!(system.contains("flow_score_informational_only"));
        assert!(system
            .to_lowercase()
            .contains("ne considere jamais flow_score_informational_only"));
        assert!(!prompt.contains("Methode obligatoire"));
        assert!(!prompt.contains("\"liquidity_score\""));
        assert!(!prompt.contains("liquidite faible"));
    }

    #[test]
    fn cache_path_uses_stable_sha256_hex() {
        let path = cache_path(Path::new("data/cache/llm"), "gemini-3-pro-preview", "abc");
        let file_name = path.file_name().unwrap().to_string_lossy();

        assert_eq!(file_name.len(), 69);
        assert!(file_name.ends_with(".json"));
        assert!(file_name
            .trim_end_matches(".json")
            .chars()
            .all(|ch| ch.is_ascii_hexdigit()));
        assert_eq!(
            path,
            cache_path(Path::new("data/cache/llm"), "gemini-3-pro-preview", "abc")
        );
    }

    #[test]
    fn extract_gemini_text_reports_prompt_feedback_block_reason() {
        let value = json!({
            "promptFeedback": {
                "blockReason": "SAFETY"
            }
        });

        let error = extract_gemini_text(&value).unwrap_err().to_string();

        assert!(error.contains("Gemini a bloque la requete"));
        assert!(error.contains("SAFETY"));
    }

    #[test]
    fn extract_gemini_text_reports_unexpected_payload_excerpt() {
        let value = json!({ "unexpected": true });

        let error = extract_gemini_text(&value).unwrap_err().to_string();

        assert!(error.contains("Payload:"));
        assert!(error.contains("unexpected"));
    }

    #[test]
    fn sanitizes_liquidity_language_from_llm_review() {
        let review = LlmScenarioReview {
            verdict: "WATCH".to_string(),
            confidence: 80,
            summary: "Liquidite faible sur ce produit.".to_string(),
            decision_drivers: vec![
                "Rendement net positif".to_string(),
                "Flow faible mais informatif seulement".to_string(),
            ],
            red_flags: vec!["liquidity risk".to_string()],
            main_risks: vec![
                "Liquidite insuffisante".to_string(),
                "Spread large".to_string(),
                "liquidity risk".to_string(),
            ],
            execution_checks: vec!["Verifier liquidite du carnet".to_string()],
            data_issues: vec!["Aucun probleme".to_string()],
            trade_plan_review: "Plan coherent, mais liquidite faible.".to_string(),
            invalidation_conditions: vec!["Liquidite insuffisante".to_string()],
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
            volatility_shock_points: -5.0,
            real_world_drift: 0.08,
            fx_target_rate: None,
            fx_stress_pct: 0.0,
            vol_spot_slope_points_per_pct: -0.50,
            exit_spread_multiplier: 1.5,
            exit_spread_delta_penalty: 0.75,
            linear_financing_rate: 0.0,
            monte_carlo_paths: 256,
            monte_carlo_max_steps: 64,
            stale_pricing_guard: false,
            paris_hour: None,
            dividends: Vec::new(),
        }
    }

    fn candidate() -> ScenarioCandidate {
        ScenarioCandidate {
            decision: "WATCH",
            score: 53.0,
            symbol: "D12QS".to_string(),
            side: "call".to_string(),
            product_family: "warrant".to_string(),
            pricing_model: "warrant_intrinsic".to_string(),
            product_type: "Warrant Call".to_string(),
            url: "https://example.test".to_string(),
            strike: 280.0,
            maturity: NaiveDate::from_ymd_opt(2027, 3, 19).unwrap(),
            maturity_label: "2027-03-19".to_string(),
            entry_price: 3.8,
            projected_price: 5.01,
            projected_bid_price: 4.97,
            stressed_projected_price: Some(4.42),
            fx_stressed_projected_price: None,
            net_return_pct: 31.36,
            stressed_net_return_pct: Some(15.8),
            fx_stressed_net_return_pct: None,
            gross_return_pct: 31.8,
            fee_drag_pct: 0.38,
            breakeven_underlying: Some(313.26),
            breakeven_distance_pct: Some(5.37),
            implied_volatility: Some(0.2837),
            volatility_used: 0.2837,
            exit_volatility: 0.2637,
            dynamic_volatility_shift_points: -2.0,
            volatility_shock_points: -5.0,
            theta_horizon_pct: Some(0.0),
            spread_pct: Some(0.53),
            probability_target_pct: Some(29.25),
            probability_breakeven_pct: Some(39.2),
            terminal_probability_target_pct: Some(18.0),
            terminal_probability_breakeven_pct: Some(25.0),
            target_zscore: Some(0.546),
            breakeven_zscore: Some(0.274),
            expected_value_pct: Some(1.68),
            sharpe_like: Some(-1.03),
            kelly_fraction_pct: Some(0.0),
            delta: Some(0.0588),
            target_delta: Some(0.0720),
            gamma: Some(0.000393),
            target_gamma: Some(0.000320),
            vega_per_vol_point: Some(0.082),
            target_vega_per_vol_point: Some(0.075),
            theta_per_day: Some(-0.005),
            target_theta_per_day: Some(-0.004),
            rho_per_rate_point: Some(0.114),
            d1: Some(0.49),
            d2: Some(0.23),
            data_quality_score: 100.0,
            liquidity_score: 45.0,
            parity: 10.0,
            fx_rate: 1.0,
            fx_exit_rate: 1.0,
            fx_stressed_exit_rate: None,
            effective_exit_spread_multiplier: 1.5,
            discrete_dividend_pv: 0.0,
            projected_reference: 280.0,
            projected_barrier: None,
            financing_drag_pct: None,
            barrier_touch_probability_pct: None,
            monte_carlo_target_first_pct: Some(28.0),
            monte_carlo_stop_first_pct: Some(18.0),
            monte_carlo_ko_pct: Some(0.0),
            monte_carlo_expected_return_pct: Some(2.1),
            monte_carlo_p05_return_pct: Some(-25.0),
            monte_carlo_p50_return_pct: Some(-3.0),
            monte_carlo_p95_return_pct: Some(42.0),
            reasons: vec![],
            warnings: vec![],
        }
    }
}
