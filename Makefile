.PHONY: dev dev-api dev-web dev-workers infra infra-down migrate test lint build clean

# Start all infrastructure (PostgreSQL, Redis, MinIO)
infra:
	docker compose up -d

# Start Temporal (separate because it's heavyweight)
temporal:
	docker compose -f infra/temporal/docker-compose.temporal.yml up -d

# Stop all infrastructure
infra-down:
	docker compose down
	docker compose -f infra/temporal/docker-compose.temporal.yml down

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

# Run all tests
test:
	cargo test --workspace
	cd apps/web && pnpm test 2>/dev/null || true

# Lint everything
lint:
	cargo clippy --workspace -- -D warnings
	cargo fmt --all -- --check
	cd apps/web && pnpm lint 2>/dev/null || true

# Build all
build:
	cargo build --release
	cd apps/web && pnpm build

# Clean build artifacts
clean:
	cargo clean
	rm -rf apps/web/.next apps/web/node_modules
