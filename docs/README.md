
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

