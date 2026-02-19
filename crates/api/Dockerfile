# ── Stage 1: Build ──
FROM rust:1.85-slim AS builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy workspace manifests first for layer caching
COPY Cargo.toml Cargo.lock ./
COPY crates/shared/Cargo.toml crates/shared/Cargo.toml
COPY crates/db/Cargo.toml crates/db/Cargo.toml
COPY crates/storage/Cargo.toml crates/storage/Cargo.toml
COPY crates/api/Cargo.toml crates/api/Cargo.toml

# Create dummy source files to cache dependency compilation
RUN mkdir -p crates/shared/src crates/db/src crates/storage/src crates/api/src && \
    echo "pub fn _dummy() {}" > crates/shared/src/lib.rs && \
    echo "pub fn _dummy() {}" > crates/db/src/lib.rs && \
    echo "fn main() {}" > crates/db/src/migrate.rs && \
    echo "pub fn _dummy() {}" > crates/storage/src/lib.rs && \
    echo "fn main() {}" > crates/api/src/main.rs

RUN cargo build --release --bin platform-api && \
    rm -rf crates/

# Copy real source code
COPY crates/ crates/

# Touch source files to invalidate cache for the actual build
RUN touch crates/shared/src/lib.rs crates/db/src/lib.rs crates/storage/src/lib.rs crates/api/src/main.rs

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
