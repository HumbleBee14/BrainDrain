# Platform

Project: An end-to-end LLM fine-tuning platform where users upload raw data, the system handles data curation and generates training data, and fine-tunes models, evaluates quality, and serves the result.

## Quick Start

```bash
make lint && make test
docker compose up -d              # Start PostgreSQL, Redis, MinIO
make migrate                      # Run database migrations
make dev-api                      # Start Rust API server
cd apps/web && pnpm install && pnpm dev  # Start frontend
```

> **Type generation:** TypeScript types are auto-generated from Rust via `ts-rs`. After changing any Rust DTO or enum, run `make typegen` to regenerate the TypeScript interfaces in `apps/web/src/lib/generated/`. This also runs automatically as part of `make test`.

---
---

## Format, Lint & Test Reference

### 🦀 Rust

| What | Command |
|---|---|
| Format check | `cargo fmt --all -- --check` |
| Format fix (auto) | `cargo fmt --all` |
| Lint (clippy) | `cargo clippy --workspace -- -D warnings` |
| Run tests | `cargo test --workspace` |
| Regenerate TS types | `cargo test --workspace export_bindings_` |

### 🐍 Python (workers)

| What | Command |
|---|---|
| Lint check | `cd apps/workers && uv run ruff check src/` |
| Format check | `cd apps/workers && uv run ruff format --check src/` |
| Format fix (auto) | `cd apps/workers && uv run ruff format src/` |
| Lint fix (auto) | `cd apps/workers && uv run ruff check --fix src/` |

### 🌐 TypeScript / Next.js

| What | Command |
|---|---|
| ESLint check | `cd apps/web && pnpm lint` |
| Type check (tsc) | `cd apps/web && pnpm type-check` |
| Build check | `cd apps/web && pnpm build` |

### 🚀 Run Everything (Makefile shortcuts)

| What | Command |
|---|---|
| All lint + format checks | `make lint` |
| All tests | `make test` |
| Full CI pass | `make lint && make test` |


---


## Documentation

| Document | Description |
|----------|-------------|
| [docs/PROJECT_FLOW.md](docs/PROJECT_FLOW.md) | **Start here** — Complete end-to-end project guide with flow diagrams |
| [docs/QUICKSTART.md](docs/QUICKSTART.md) | Setup, run, test, and deploy commands (concise & copy-pasteable) |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | System architecture, technical decision records, component registry |
| [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) | Development tracker — phase-by-phase build history |
| [docs/RESEARCH.md](docs/RESEARCH.md) | LLM fine-tuning landscape research and analysis |