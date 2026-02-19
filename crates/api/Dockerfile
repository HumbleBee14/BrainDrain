# ── Stage 1: Build ──
FROM rust:1.85-slim AS builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy entire workspace (BuildKit layer caching + CI gha cache handle efficiency)
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/

RUN cargo build --release --bin platform-api

# ── Stage 2: Runtime ──
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

RUN useradd -r -s /bin/false appuser

COPY --from=builder /app/target/release/platform-api /usr/local/bin/platform-api

# Copy migrations for runtime migration execution
COPY crates/db/src/migrations/ /app/migrations/

USER appuser

ENV RUST_LOG=info
EXPOSE 8000

CMD ["platform-api"]
