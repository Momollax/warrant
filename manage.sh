#!/usr/bin/env bash
# manage.sh — Gestion du container Docker warrant-fetcher
set -euo pipefail

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
    local log_level="${1:-info}"
    local tickers="${2:-}"   # ex: "0ABC.PA,0XYZ.PA" — vide = tickers du code source

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
        ${tickers:+-e "TICKERS=${tickers}"} \
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
        cmd_run "${1:-info}"
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
    local ticker="${1:-AAPL}"
    info "Test de connectivité — 1 cycle avec le ticker '${ticker}'…"
    info "(Ctrl+C pour interrompre)"
    docker run --rm \
        -e "RUST_LOG=info" \
        -e "TICKERS=${ticker}" \
        -e "MAX_CYCLES=1" \
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
    cmd_run "${1:-info}"
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
  ${GREEN}test${NC}  [ticker]               Teste la connectivité (1 cycle, défaut: AAPL)
  ${GREEN}clean${NC}                        Supprime le container et l'image (confirmation)
  ${GREEN}rebuild${NC} [log] [tickers]      Stop + clean image + build + run
  ${GREEN}help${NC}                         Affiche cette aide

${BOLD}Exemples :${NC}
  $0 build
  $0 run info "0ABC.PA,0XYZ.PA"
  $0 test AAPL
  $0 logs 100
  $0 status
  $0 rebuild
"
}

# ── Dispatcher ────────────────────────────────────────────────────────────────
case "${1:-help}" in
    build)   cmd_build ;;
    run)     cmd_run     "${2:-info}" ;;
    stop)    cmd_stop ;;
    restart) cmd_restart "${2:-info}" ;;
    logs)    cmd_logs    "${2:-50}" ;;
    status)  cmd_status ;;
    shell)   cmd_shell ;;
    clean)   cmd_clean ;;
    test)    cmd_test    "${2:-AAPL}" ;;
    rebuild) cmd_rebuild "${2:-info}" ;;
    help|--help|-h) cmd_help ;;
    *)
        error "Commande inconnue : '${1}'"
        cmd_help
        exit 1
        ;;
esac
