# ── Stage 1: Build ──
FROM rust:1-slim AS builder

# curl is required by utoipa-swagger-ui's build script to fetch the Swagger UI bundle.
RUN apt-get update && apt-get install -y pkg-config libssl-dev curl && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy entire workspace (BuildKit layer caching + CI gha cache handle efficiency)
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/

RUN cargo build --release --bin platform-api --bin migrate

# ── Stage 2: Runtime ──
FROM debian:trixie-slim AS runtime

RUN apt-get update && apt-get install -y ca-certificates curl && rm -rf /var/lib/apt/lists/*

RUN useradd -r -s /bin/false appuser

COPY --from=builder /app/target/release/platform-api /usr/local/bin/platform-api
COPY --from=builder /app/target/release/migrate /usr/local/bin/platform-migrate

# Copy migrations for runtime migration execution
COPY crates/db/src/migrations/ /app/migrations/

USER appuser

ENV RUST_LOG=info
EXPOSE 8000

HEALTHCHECK --interval=15s --timeout=3s --retries=3 \
  CMD curl -fsS http://localhost:8000/health || exit 1

CMD ["platform-api"]
