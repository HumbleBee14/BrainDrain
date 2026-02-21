.PHONY: dev dev-api dev-web dev-workers infra infra-down migrate test lint build clean typegen observability

# Start all infrastructure (PostgreSQL, Redis, MinIO)
infra:
	docker network create platform-net 2>/dev/null || true
	docker compose up -d

# Start Temporal (separate because it's heavyweight)
temporal:
	docker compose -f infra/temporal/docker-compose.temporal.yml up -d

# Start observability stack (OTEL Collector, Prometheus, Tempo, Loki, Grafana)
observability:
	docker compose -f infra/otel/docker-compose.otel.yml up -d

# Stop all infrastructure
infra-down:
	docker compose down
	docker compose -f infra/temporal/docker-compose.temporal.yml down
	docker compose -f infra/otel/docker-compose.otel.yml down

# Run database migrations
migrate:
	cargo run -p platform-db --bin migrate

# Start API server (Rust)
dev-api:
	cargo run -p platform-api

# Start frontend (Next.js)
dev-web:
	cd apps/web && pnpm dev

# Start Temporal workers (Python)
dev-workers:
	cd apps/workers && uv run python -m src.worker

# Generate TypeScript types from Rust (ts-rs)
typegen:
	cargo test --workspace export_bindings_ -- --exact && echo "TypeScript types regenerated in apps/web/src/lib/generated/"

# Run all tests
test:
	cargo test --workspace
	cd apps/web && pnpm type-check

# Lint everything (Rust + Frontend + Python) — fails fast on first error
lint:
	cargo fmt --all -- --check
	cargo clippy --workspace -- -D warnings
	cd apps/web && pnpm lint
	cd apps/web && pnpm type-check
	cd apps/workers && uv run ruff check src/
	cd apps/workers && uv run ruff format --check src/

# Build all
build:
	cargo build --release
	cd apps/web && pnpm build

# Clean build artifacts
clean:
	cargo clean
	rm -rf apps/web/.next apps/web/node_modules
