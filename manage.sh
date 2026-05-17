#!/usr/bin/env bash
# manage.sh — Gestion du container Docker warrant-fetcher
set -euo pipefail

load_env_file() {
    local env_file="${1:-.env}"
    local line key value

    [[ -f "${env_file}" ]] || return 0

    while IFS= read -r line || [[ -n "${line}" ]]; do
        line="${line%$'\r'}"
        [[ -z "${line}" || "${line}" =~ ^[[:space:]]*# ]] && continue
        [[ "${line}" != *=* ]] && continue

        key="${line%%=*}"
        value="${line#*=}"
        key="${key%"${key##*[![:space:]]}"}"
        key="${key#"${key%%[![:space:]]*}"}"

        [[ "${key}" =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]] || continue
        [[ -v "${key}" ]] && continue

        if [[ "${value}" =~ ^\".*\"$ || "${value}" =~ ^\'.*\'$ ]]; then
            value="${value:1:${#value}-2}"
        fi
        export "${key}=${value}"
    done < "${env_file}"
}

load_env_file ".env"

IMAGE="warrant-fetcher"
CONTAINER="warrant-fetcher"

# ── Couleurs ──────────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

info()    { echo -e "${CYAN}[INFO]${NC}  $*"; }
success() { echo -e "${GREEN}[OK]${NC}    $*"; }
warn()    { echo -e "${YELLOW}[WARN]${NC}  $*"; }
error()   { echo -e "${RED}[ERR]${NC}   $*" >&2; }

# ── Helpers ───────────────────────────────────────────────────────────────────
is_running() {
    docker ps --filter "name=^/${CONTAINER}$" --format '{{.Names}}' 2>/dev/null | grep -q "^${CONTAINER}$"
}

is_stopped() {
    docker ps -a --filter "name=^/${CONTAINER}$" --format '{{.Names}}' 2>/dev/null | grep -q "^${CONTAINER}$"
}

require_running() {
    if ! is_running; then
        error "Le container '${CONTAINER}' n'est pas en cours d'exécution."
        echo "  → Lance-le avec : $0 run"
        exit 1
    fi
}

# ── Commandes ─────────────────────────────────────────────────────────────────

cmd_build() {
    info "Construction de l'image '${IMAGE}'…"
    docker build -t "${IMAGE}" .
    success "Image '${IMAGE}' construite."
}

cmd_run() {
    local log_level="${1:-${RUST_LOG:-info}}"
    local display_mode="${DISPLAY_MODE:-headless}"
    local max_cycles="${MAX_CYCLES:-}"
    local tickers="${2:-}"   # ex: "0ABC.PA,0XYZ.PA" — vide = tickers du code source

    if [[ -z "${tickers}" ]]; then tickers="${TICKERS:-}"; fi

    if is_running; then
        warn "Le container tourne déjà. Utilise '$0 restart' pour le relancer."
        exit 0
    fi

    # Supprime un ancien container stoppé s'il existe
    if is_stopped; then
        info "Suppression de l'ancien container stoppé…"
        docker rm "${CONTAINER}" > /dev/null
    fi

    info "Démarrage du container '${CONTAINER}' (RUST_LOG=${log_level})…"
    docker run -d \
        --name "${CONTAINER}" \
        --restart unless-stopped \
        -e "RUST_LOG=${log_level}" \
        -e "DISPLAY_MODE=${display_mode}" \
        ${tickers:+-e "TICKERS=${tickers}"} \
        ${max_cycles:+-e "MAX_CYCLES=${max_cycles}"} \
        "${IMAGE}"
    success "Container démarré. Logs : $0 logs"
}

cmd_stop() {
    if ! is_running; then
        warn "Le container n'est pas en cours d'exécution."
        exit 0
    fi
    info "Arrêt du container '${CONTAINER}'…"
    docker stop "${CONTAINER}" > /dev/null
    success "Container arrêté."
}

cmd_restart() {
    if is_running; then
        info "Redémarrage du container '${CONTAINER}'…"
        docker restart "${CONTAINER}" > /dev/null
        success "Container redémarré."
    else
        warn "Container non démarré, lancement…"
        cmd_run "${1:-}" "${2:-}"
    fi
}

cmd_logs() {
    local lines="${1:-50}"
    require_running
    info "Affichage des logs (${lines} dernières lignes, Ctrl+C pour quitter)…"
    docker logs -f --tail="${lines}" "${CONTAINER}"
}

cmd_status() {
    echo -e "\n${BOLD}── Image ────────────────────────────────────────────${NC}"
    docker images "${IMAGE}" 2>/dev/null || echo "  (aucune image)"

    echo -e "\n${BOLD}── Container ────────────────────────────────────────${NC}"
    docker ps -a --filter "name=^/${CONTAINER}$" \
        --format "table {{.Names}}\t{{.Status}}\t{{.CreatedAt}}" 2>/dev/null \
        || echo "  (aucun container)"

    echo -e "\n${BOLD}── Utilisation des ressources ───────────────────────${NC}"
    if is_running; then
        docker stats "${CONTAINER}" --no-stream \
            --format "table {{.Name}}\t{{.CPUPerc}}\t{{.MemUsage}}\t{{.NetIO}}"
    else
        echo "  (container stoppé)"
    fi
    echo
}

cmd_shell() {
    require_running
    info "Ouverture d'un shell dans le container…"
    docker exec -it "${CONTAINER}" /bin/sh
}

cmd_clean() {
    warn "Cela va supprimer le container ET l'image '${IMAGE}'."
    read -r -p "Confirmer ? [y/N] " confirm
    [[ "${confirm}" =~ ^[Yy]$ ]] || { info "Annulé."; exit 0; }

    if is_running; then
        info "Arrêt du container…"
        docker stop "${CONTAINER}" > /dev/null
    fi
    if is_stopped; then
        info "Suppression du container…"
        docker rm "${CONTAINER}" > /dev/null
    fi
    if docker image inspect "${IMAGE}" &>/dev/null; then
        info "Suppression de l'image…"
        docker rmi "${IMAGE}" > /dev/null
    fi
    success "Nettoyage terminé."
}

cmd_test() {
    local ticker="${1:-${TICKERS:-RMS.PA}}"
    info "Test de connectivité — 1 cycle avec le ticker '${ticker}'…"
    info "(Ctrl+C pour interrompre)"
    docker run --rm \
        -e "RUST_LOG=info" \
        -e "DISPLAY_MODE=headless" \
        -e "TICKERS=${ticker}" \
        -e "MAX_CYCLES=1" \
        "${IMAGE}"
}

cmd_test_unit() {
    info "Lancement des tests unitaires Rust via Docker..."
    docker run --rm \
        -v "${PWD}:/app" \
        -w /app \
        rust:latest \
        cargo test
}

cmd_orats_test() {
    local ticker="${1:-${UNDERLYING_TICKER:-RMS.PA}}"
    local orats_base_url="${ORATS_BASE_URL:-https://api.orats.io}"
    info "Test ORATS pour '${ticker}' avec rotation des cles configurees..."
    docker run --rm \
        -e "ORATS_BASE_URL=${orats_base_url}" \
        ${ORATS_API_KEY:+-e "ORATS_API_KEY=${ORATS_API_KEY}"} \
        "${IMAGE}" \
        ./warrant_fetcher orats-test "${ticker}"
}

cmd_polygon_test() {
    local ticker="${1:-${UNDERLYING_TICKER:-RMS.PA}}"
    local polygon_base_url="${POLYGON_BASE_URL:-https://api.polygon.io}"
    info "Test Polygon pour '${ticker}' avec rotation des cles configurees..."
    docker run --rm \
        -e "POLYGON_BASE_URL=${polygon_base_url}" \
        ${POLYGON_API_KEY:+-e "POLYGON_API_KEY=${POLYGON_API_KEY}"} \
        "${IMAGE}" \
        ./warrant_fetcher polygon-test "${ticker}"
}

cmd_discover() {
    local underlying="${1:-${DISCOVER_UNDERLYING:-hermes}}"
    local limit="${2:-${DISCOVER_LIMIT:-}}"
    info "Recherche des produits structurés Euronext pour '${underlying}'…"
    docker run --rm \
        -e "DISCOVER_UNDERLYING=${underlying}" \
        ${limit:+-e "DISCOVER_LIMIT=${limit}"} \
        "${IMAGE}"
}

cmd_analyze() {
    local underlying="${1:-${ANALYZE_UNDERLYING:-hermes}}"
    local underlying_ticker="${2:-${UNDERLYING_TICKER:-RMS.PA}}"
    local limit="${3:-${ANALYZE_LIMIT:-}}"
    local boursorama_enrich="${BOURSORAMA_ENRICH:-0}"
    info "Analyse des produits '${underlying}' avec spot '${underlying_ticker}'…"
    docker run --rm \
        -e "ANALYZE_UNDERLYING=${underlying}" \
        -e "UNDERLYING_TICKER=${underlying_ticker}" \
        -e "BOURSORAMA_ENRICH=${boursorama_enrich}" \
        ${limit:+-e "ANALYZE_LIMIT=${limit}"} \
        "${IMAGE}"
}

cmd_opportunities() {
    local underlying="${1:-${OPPORTUNITY_UNDERLYING:-hermes}}"
    local underlying_ticker="${2:-${UNDERLYING_TICKER:-RMS.PA}}"
    local limit="${3:-${OPPORTUNITY_LIMIT:-}}"
    local side="${4:-${OPPORTUNITY_SIDE:-all}}"
    local boursorama_enrich="${BOURSORAMA_ENRICH:-0}"
    local validate_boursorama="${OPPORTUNITY_VALIDATE_BOURSORAMA:-1}"
    local validate_limit="${OPPORTUNITY_VALIDATE_LIMIT:-500}"
    local min_gap_pct="${OPPORTUNITY_MIN_GAP_PCT:-1}"
    local require_validated_price="${OPPORTUNITY_REQUIRE_VALIDATED_PRICE:-1}"
    local option_risk_free_rate="${OPTION_RISK_FREE_RATE:-0.045}"
    local option_dividend_yield="${OPTION_DIVIDEND_YIELD:-0.005}"
    local option_iv_signal_threshold="${OPTION_IV_SIGNAL_THRESHOLD:-0.03}"
    local orats_base_url="${ORATS_BASE_URL:-https://api.orats.io}"
    local polygon_base_url="${POLYGON_BASE_URL:-https://api.polygon.io}"
    local opportunity_debug="${OPPORTUNITY_DEBUG:-0}"
    local opportunity_debug_every="${OPPORTUNITY_DEBUG_EVERY:-25}"
    local requested_format="${OPPORTUNITY_FORMAT:-}"
    local interactive_format="${requested_format:-tui}"
    if [[ -z "${requested_format}" && "${opportunity_debug}" =~ ^(1|true|yes|on)$ ]]; then
        interactive_format="table"
    fi
    info "Recherche de décorrélations relatives pour '${underlying}' avec spot '${underlying_ticker}'…"
    if [[ -t 0 && -t 1 ]]; then
        docker run --rm -it \
            -e "TERM=${TERM:-xterm-256color}" \
            -e "OPPORTUNITY_UNDERLYING=${underlying}" \
            -e "UNDERLYING_TICKER=${underlying_ticker}" \
            -e "OPPORTUNITY_FORMAT=${interactive_format}" \
            -e "OPPORTUNITY_SIDE=${side}" \
            -e "BOURSORAMA_ENRICH=${boursorama_enrich}" \
            -e "OPPORTUNITY_VALIDATE_BOURSORAMA=${validate_boursorama}" \
            -e "OPPORTUNITY_VALIDATE_LIMIT=${validate_limit}" \
            -e "OPPORTUNITY_MIN_GAP_PCT=${min_gap_pct}" \
            -e "OPPORTUNITY_REQUIRE_VALIDATED_PRICE=${require_validated_price}" \
            -e "OPTION_RISK_FREE_RATE=${option_risk_free_rate}" \
            -e "OPTION_DIVIDEND_YIELD=${option_dividend_yield}" \
            -e "OPTION_IV_SIGNAL_THRESHOLD=${option_iv_signal_threshold}" \
            -e "ORATS_BASE_URL=${orats_base_url}" \
            -e "POLYGON_BASE_URL=${polygon_base_url}" \
            -e "OPPORTUNITY_DEBUG=${opportunity_debug}" \
            -e "OPPORTUNITY_DEBUG_EVERY=${opportunity_debug_every}" \
            ${ORATS_API_KEY:+-e "ORATS_API_KEY=${ORATS_API_KEY}"} \
            ${FRED_API_KEY:+-e "FRED_API_KEY=${FRED_API_KEY}"} \
            ${POLYGON_API_KEY:+-e "POLYGON_API_KEY=${POLYGON_API_KEY}"} \
            ${BROWSER:+-e "BROWSER=${BROWSER}"} \
            ${limit:+-e "OPPORTUNITY_LIMIT=${limit}"} \
            "${IMAGE}"
    else
        docker run --rm \
            -e "OPPORTUNITY_UNDERLYING=${underlying}" \
            -e "UNDERLYING_TICKER=${underlying_ticker}" \
            -e "OPPORTUNITY_FORMAT=${requested_format:-table}" \
            -e "OPPORTUNITY_SIDE=${side}" \
            -e "BOURSORAMA_ENRICH=${boursorama_enrich}" \
            -e "OPPORTUNITY_VALIDATE_BOURSORAMA=${validate_boursorama}" \
            -e "OPPORTUNITY_VALIDATE_LIMIT=${validate_limit}" \
            -e "OPPORTUNITY_MIN_GAP_PCT=${min_gap_pct}" \
            -e "OPPORTUNITY_REQUIRE_VALIDATED_PRICE=${require_validated_price}" \
            -e "OPTION_RISK_FREE_RATE=${option_risk_free_rate}" \
            -e "OPTION_DIVIDEND_YIELD=${option_dividend_yield}" \
            -e "OPTION_IV_SIGNAL_THRESHOLD=${option_iv_signal_threshold}" \
            -e "ORATS_BASE_URL=${orats_base_url}" \
            -e "POLYGON_BASE_URL=${polygon_base_url}" \
            -e "OPPORTUNITY_DEBUG=${opportunity_debug}" \
            -e "OPPORTUNITY_DEBUG_EVERY=${opportunity_debug_every}" \
            ${ORATS_API_KEY:+-e "ORATS_API_KEY=${ORATS_API_KEY}"} \
            ${FRED_API_KEY:+-e "FRED_API_KEY=${FRED_API_KEY}"} \
            ${POLYGON_API_KEY:+-e "POLYGON_API_KEY=${POLYGON_API_KEY}"} \
            ${BROWSER:+-e "BROWSER=${BROWSER}"} \
            ${limit:+-e "OPPORTUNITY_LIMIT=${limit}"} \
            "${IMAGE}"
    fi
}

cmd_opportunities_csv() {
    local underlying="${1:-${OPPORTUNITY_UNDERLYING:-hermes}}"
    local underlying_ticker="${2:-${UNDERLYING_TICKER:-RMS.PA}}"
    local limit="${3:-${OPPORTUNITY_LIMIT:-}}"
    local side="${4:-${OPPORTUNITY_SIDE:-all}}"
    local boursorama_enrich="${BOURSORAMA_ENRICH:-0}"
    local validate_boursorama="${OPPORTUNITY_VALIDATE_BOURSORAMA:-1}"
    local validate_limit="${OPPORTUNITY_VALIDATE_LIMIT:-500}"
    local min_gap_pct="${OPPORTUNITY_MIN_GAP_PCT:-1}"
    local require_validated_price="${OPPORTUNITY_REQUIRE_VALIDATED_PRICE:-1}"
    local option_risk_free_rate="${OPTION_RISK_FREE_RATE:-0.045}"
    local option_dividend_yield="${OPTION_DIVIDEND_YIELD:-0.005}"
    local option_iv_signal_threshold="${OPTION_IV_SIGNAL_THRESHOLD:-0.03}"
    local orats_base_url="${ORATS_BASE_URL:-https://api.orats.io}"
    local polygon_base_url="${POLYGON_BASE_URL:-https://api.polygon.io}"
    local opportunity_debug="${OPPORTUNITY_DEBUG:-0}"
    local opportunity_debug_every="${OPPORTUNITY_DEBUG_EVERY:-25}"
    docker run --rm \
        -e "OPPORTUNITY_UNDERLYING=${underlying}" \
        -e "UNDERLYING_TICKER=${underlying_ticker}" \
        -e "OPPORTUNITY_FORMAT=csv" \
        -e "OPPORTUNITY_SIDE=${side}" \
        -e "BOURSORAMA_ENRICH=${boursorama_enrich}" \
        -e "OPPORTUNITY_VALIDATE_BOURSORAMA=${validate_boursorama}" \
        -e "OPPORTUNITY_VALIDATE_LIMIT=${validate_limit}" \
        -e "OPPORTUNITY_MIN_GAP_PCT=${min_gap_pct}" \
        -e "OPPORTUNITY_REQUIRE_VALIDATED_PRICE=${require_validated_price}" \
        -e "OPTION_RISK_FREE_RATE=${option_risk_free_rate}" \
        -e "OPTION_DIVIDEND_YIELD=${option_dividend_yield}" \
        -e "OPTION_IV_SIGNAL_THRESHOLD=${option_iv_signal_threshold}" \
        -e "ORATS_BASE_URL=${orats_base_url}" \
        -e "POLYGON_BASE_URL=${polygon_base_url}" \
        -e "OPPORTUNITY_DEBUG=${opportunity_debug}" \
        -e "OPPORTUNITY_DEBUG_EVERY=${opportunity_debug_every}" \
        ${ORATS_API_KEY:+-e "ORATS_API_KEY=${ORATS_API_KEY}"} \
        ${FRED_API_KEY:+-e "FRED_API_KEY=${FRED_API_KEY}"} \
        ${POLYGON_API_KEY:+-e "POLYGON_API_KEY=${POLYGON_API_KEY}"} \
        ${limit:+-e "OPPORTUNITY_LIMIT=${limit}"} \
        "${IMAGE}"
}

cmd_rebuild() {
    info "Rebuild complet (stop → clean image → build → run)…"
    if is_running; then cmd_stop; fi
    if is_stopped; then docker rm "${CONTAINER}" > /dev/null 2>&1 || true; fi
    if docker image inspect "${IMAGE}" &>/dev/null; then
        docker rmi "${IMAGE}" > /dev/null
    fi
    cmd_build
    cmd_run "${1:-}"
}

cmd_help() {
    echo -e "
${BOLD}Usage :${NC}  $0 <commande> [options]

${BOLD}Commandes :${NC}
  ${GREEN}build${NC}                        Construit l'image Docker
  ${GREEN}run${NC}   [log] [tickers]        Démarre en arrière-plan
                               log     : info (défaut) | debug | warn | error
                               tickers : liste CSV  ex: \"0ABC.PA,0XYZ.PA\"
  ${GREEN}stop${NC}                         Arrête le container
  ${GREEN}restart${NC} [log] [tickers]      Redémarre le container
  ${GREEN}logs${NC}  [n_lignes]             Suit les logs en temps réel (défaut : 50)
  ${GREEN}status${NC}                       Affiche l'état, l'image et les ressources
  ${GREEN}shell${NC}                        Ouvre un shell interactif dans le container
  ${GREEN}test${NC}  [ticker]               Teste la connectivité (1 cycle, défaut: RMS.PA)
  ${GREEN}discover${NC} [underlying] [limit] Liste les produits Euronext (défaut: hermes)
  ${GREEN}test-unit${NC}                    Lance les tests unitaires Rust
  ${GREEN}orats-test${NC} [ticker]          Teste ORATS avec rotation des cles API
  ${GREEN}polygon-test${NC} [ticker]        Teste Polygon avec rotation des cles API
  ${GREEN}analyze${NC} [underlying] [spot] [limit]
                               Base triée avec spot, call/put, strike, maturité, prix
  ${GREEN}opportunities${NC} [underlying] [spot] [limit] [all|call|put]
                               UI claire des candidats décorrélés
  ${GREEN}opportunities-csv${NC} [underlying] [spot] [limit] [all|call|put]
                               Export CSV des candidats décorrélés
  ${GREEN}clean${NC}                        Supprime le container et l'image (confirmation)
  ${GREEN}rebuild${NC} [log] [tickers]      Stop + clean image + build + run
  ${GREEN}help${NC}                         Affiche cette aide

${BOLD}.env :${NC}
  Les variables de .env sont chargees comme valeurs par defaut.
  Une variable deja exportee dans le shell garde la priorite.
  Ex: OPPORTUNITY_LIMIT, OPPORTUNITY_SIDE, UNDERLYING_TICKER,
      BOURSORAMA_ENRICH, OPPORTUNITY_VALIDATE_BOURSORAMA,
      OPPORTUNITY_VALIDATE_LIMIT, OPPORTUNITY_MIN_GAP_PCT,
      OPPORTUNITY_REQUIRE_VALIDATED_PRICE, OPTION_RISK_FREE_RATE,
      OPTION_DIVIDEND_YIELD, OPTION_IV_SIGNAL_THRESHOLD,
      OPPORTUNITY_DEBUG, OPPORTUNITY_DEBUG_EVERY,
      ORATS_API_KEY, POLYGON_API_KEY, BROWSER.

${BOLD}Exemples :${NC}
  $0 build
  $0 run info "0ABC.PA,0XYZ.PA"
  $0 test RMS.PA
  $0 test-unit
  $0 orats-test RMS.PA
  $0 polygon-test RMS.PA
  $0 discover hermes 20
  $0 analyze hermes RMS.PA 50
  $0 opportunities hermes RMS.PA 500 call
  $0 opportunities-csv hermes RMS.PA 500 put
  $0 logs 100
  $0 status
  $0 rebuild
"
}

# ── Dispatcher ────────────────────────────────────────────────────────────────
case "${1:-help}" in
    build)   cmd_build ;;
    run)     cmd_run     "${2:-}" "${3:-}" ;;
    stop)    cmd_stop ;;
    restart) cmd_restart "${2:-}" "${3:-}" ;;
    logs)    cmd_logs    "${2:-50}" ;;
    status)  cmd_status ;;
    shell)   cmd_shell ;;
    clean)   cmd_clean ;;
    test)    cmd_test    "${2:-}" ;;
    test-unit) cmd_test_unit ;;
    orats-test) cmd_orats_test "${2:-}" ;;
    polygon-test) cmd_polygon_test "${2:-}" ;;
    discover) cmd_discover "${2:-}" "${3:-}" ;;
    analyze) cmd_analyze "${2:-}" "${3:-}" "${4:-}" ;;
    opportunities) cmd_opportunities "${2:-}" "${3:-}" "${4:-}" "${5:-}" ;;
    opportunities-csv) cmd_opportunities_csv "${2:-}" "${3:-}" "${4:-}" "${5:-}" ;;
    rebuild) cmd_rebuild "${2:-}" ;;
    help|--help|-h) cmd_help ;;
    *)
        error "Commande inconnue : '${1}'"
        cmd_help
        exit 1
        ;;
esac
