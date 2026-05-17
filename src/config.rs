/// Tickers Yahoo Finance des warrants Apple sur Euronext Paris.
/// Format : "<ticker>.PA" pour Paris, "<ticker>.DE" pour Xetra.
///
/// Surchargeable via la variable d'environnement TICKERS (séparés par des virgules) :
///   TICKERS=0ABC.PA,0XYZ.PA ./manage.sh run
pub const WARRANT_TICKERS: &[&str] = &[
    "0796T.PA", // Inline Warrant APPLE – Citigroup  (PUT-like, baissier) — EXEMPLE, peut être expiré
    "4739C.PA", // Warrant APPLE 115P – Commerzbank  (PUT)               — EXEMPLE, peut être expiré
];

/// Intervalle de rafraîchissement en secondes (Yahoo Finance impose ~15 min de délai légal)
pub const REFRESH_INTERVAL_SECS: u64 = 60;

/// Nombre de cycles de récupération (0 = infini).
/// Surchargeable via la variable d'environnement MAX_CYCLES.
pub const MAX_CYCLES: u32 = 0;

/// Lit les tickers depuis la variable d'environnement TICKERS (séparés par des virgules)
/// ou utilise WARRANT_TICKERS si la variable n'est pas définie.
pub fn resolve_tickers() -> Vec<String> {
    if let Ok(env_val) = std::env::var("TICKERS") {
        let tickers: Vec<String> = env_val
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if !tickers.is_empty() {
            return tickers;
        }
    }
    WARRANT_TICKERS.iter().map(|s| s.to_string()).collect()
}

/// Lit MAX_CYCLES depuis la variable d'environnement MAX_CYCLES ou utilise la constante.
pub fn resolve_max_cycles() -> u32 {
    std::env::var("MAX_CYCLES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(MAX_CYCLES)
}
