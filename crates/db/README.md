# crates/db

**Database models and migrations for PostgreSQL via SQLx.**

| | |
|---|---|
| Language | Rust |
| Type | Library crate + `migrate` binary |
| Database | PostgreSQL 16 |
| ORM | SQLx (compile-time checked SQL, async) |
| Depends on | `crates/shared` |
| Used by | `crates/api` |

## What's Inside

| File | Purpose |
|---|---|
| `models.rs` | 9 `FromRow` structs: `Tenant`, `Project`, `Document`, `Dataset`, `TrainingJob`, `Model`, `Evaluation`, `ApiKey`, `BillingEvent` |
| `migrations/001_initial_schema.sql` | Full schema — 9 tables, 20+ indexes, Row-Level Security (RLS), `updated_at` triggers |
| `lib.rs` | `create_pool()` — connection pool factory; `run_migrations()` — applies pending migrations |
| `migrate.rs` | Standalone binary to run migrations without starting the API server |

## Key Decisions

- **All tables have `tenant_id`** — multi-tenancy enforced at the schema level
- **RLS enabled** on all tenant-scoped tables
- **UUID primary keys**, `TIMESTAMPTZ` timestamps, `JSONB` for flexible fields
- **Soft deletes** via `deleted_at` on projects
- Cost fields use `f64` (not BigDecimal) for simplicity

## Usage

### As a Library (used by `crates/api`)

```toml
[dependencies]
platform-db = { workspace = true }
```

```rust
use platform_db::{create_pool, run_migrations};
use platform_db::models::Project;

let pool = create_pool("postgresql://...").await?;
run_migrations(&pool).await?;
```

### Run Migrations

```bash
# Automatic — API runs migrations on startup
make dev-api

# Standalone binary (requires DATABASE_URL env var)
cargo run --bin migrate

# Via Makefile
make migrate
```

### Required Environment Variables

| Variable | Example | Description |
|---|---|---|
| `DATABASE_URL` | `postgresql://platform:platform@localhost:5432/platform` | PostgreSQL connection string |

### Run Tests

```bash
cargo test -p platform-db
```
