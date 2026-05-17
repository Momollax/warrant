# ── Stage 1 : compilation ────────────────────────────────────────────────────
# ratatui 0.28 requiert Rust >= 1.88
# Cible musl → binaire statique, indépendant de la version de glibc du runtime
FROM rust:latest AS builder

WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    musl-tools \
    && rustup target add x86_64-unknown-linux-musl \
    && rm -rf /var/lib/apt/lists/*

# Copie le manifeste pour mettre les dépendances en cache Docker
COPY Cargo.toml ./

# Pré-compilation des dépendances (couche reconstruite uniquement si Cargo.toml change)
RUN mkdir -p src && echo 'fn main() {}' > src/main.rs \
    && cargo build --release --target x86_64-unknown-linux-musl \
    && rm -f target/x86_64-unknown-linux-musl/release/deps/warrant_fetcher*

# Copie le vrai code source
COPY src ./src

# Compilation finale (binaire statique musl)
RUN cargo build --release --target x86_64-unknown-linux-musl

# ── Stage 2 : image minimale d'exécution ─────────────────────────────────────
# Alpine utilise musl — compatible avec notre binaire statique
FROM alpine:latest AS runtime

# Certificats TLS nécessaires pour les requêtes HTTPS vers Yahoo Finance
RUN apk add --no-cache ca-certificates

WORKDIR /app

COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/warrant_fetcher .

ENV RUST_LOG=info

CMD ["./warrant_fetcher"]
