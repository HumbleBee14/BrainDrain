# Project Context

## What We Are Building

An LLM fine-tuning platform where anyone can upload documents, generate training data, fine-tune models, evaluate quality, and deploy — all without ML expertise. The user uploads files, the platform handles everything else.

**Core pipeline:** Upload → Parse → Refine (synthetic data) → Train (LoRA/QLoRA) → Evaluate → Deploy

## Architecture

- **Monorepo** with 3 independently deployable services (own Dockerfiles, own containers, zero runtime coupling)
- **Rust (Axum)** for all infrastructure: API gateway, file upload, database, Redis, S3, auth
- **Python** only where ML libraries force it: training (Unsloth/TRL), data generation (distilabel), parsing (MinerU)
- **TypeScript (Next.js 15)** for the frontend dashboard
- **Temporal** for durable workflow orchestration (retry, observability)
- **PostgreSQL** for data, **Redis** for cache/streaming, **S3/MinIO** for object storage

## Key Files

| What | Where |
|---|---|
| Rust workspace root | `Cargo.toml` |
| API server entry | `crates/api/src/main.rs` |
| API routes | `crates/api/src/routes/` |
| Service layer | `crates/api/src/services/` |
| Repository layer | `crates/api/src/repositories/` |
| DB models | `crates/db/src/models.rs` |
| DB migrations | `crates/db/src/migrations/` |
| Shared enums | `crates/shared/src/enums.rs` |
| Shared types | `crates/shared/src/types.rs` |
| Storage trait | `crates/storage/src/lib.rs` |
| Generated TS types | `apps/web/src/lib/generated/` |
| ts-rs config | `.cargo/config.toml` |
| Frontend pages | `apps/web/src/app/` |
| ML workflows | `apps/workers/src/workflows/` |
| ML activities | `apps/workers/src/activities/` |
| Full docs | `docs/DEVELOPMENT.md` |

## Development Phase

Phase 0 (Foundation) is complete. Current work is on Phase 1 (Data Pipeline) and beyond.

---

# Development Rules

## Naming and Branding

- **NEVER hardcode brand names** in source code. Internal code uses generic names (`platform-api`, `platform-db`, etc.)
- Brand/product name comes from `NEXT_PUBLIC_APP_NAME` env var in the frontend and `APP_NAME` in the API config
- Crate names, package names, function names, variable names — all generic and technical
- Only `.env` files and docs reference any brand name

## Architecture Patterns

### Rust API: Route → Service → Repository

```
Route (thin)       → extract auth, validate DTO, return response
Service (logic)    → orchestrate repos, S3, Redis, validation rules
Repository (data)  → pure SQL queries via SQLx, always requires tenant_id
```

- Routes should be thin. No business logic in routes.
- Services take `&PgPool` / `impl ObjectStorage` — never the full AppState
- Every database query MUST include `tenant_id` in the WHERE clause. No exceptions. This is the multi-tenancy enforcement layer.

### Trait Abstractions

- `ObjectStorage` trait for S3 — implementations are swappable
- Services depend on traits, not concrete types
- This allows testing with mocks and swapping backends without changing business logic

### Type Safety: Rust → TypeScript (ts-rs)

- **Rust is the single source of truth** for all API types. TypeScript is auto-generated via `ts-rs` v12.
- All DTOs and shared enums have `#[derive(TS)]` + `#[ts(export)]`. Running `cargo test` generates `.ts` files into `apps/web/src/lib/generated/`.
- DTO enum fields (status, method, mode) use proper Rust enums, not `String`. This generates TypeScript union types (e.g., `"pending" | "training" | "completed"`).
- `#[ts(optional)]` on request `Option<T>` fields generates `field?: T` instead of `field: T | null`.
- `api-client.ts` imports from generated types with aliases (e.g., `ProjectResponse as Project`).
- DB models keep `String` for status fields (sqlx `FromRow` requires it). Conversion happens at the DTO boundary via `.parse().unwrap_or(Default)`.
- After changing a Rust DTO or enum, run `make typegen` to regenerate TypeScript.

### Error Handling

- `AppError` enum → `IntoResponse` → consistent JSON envelope `{"error":{"code":"...","message":"..."}}`
- Internal errors (DB, storage, panics) NEVER leak details to clients
- Use `thiserror` for error types, `anyhow` for ad-hoc internal errors

## Code Quality Standards

### DO

- Write small, focused functions. If a function is more than 40 lines, consider splitting.
- Use strong types — enums over strings, newtypes over primitives where it adds clarity
- Keep dependencies minimal. Check if the standard library or existing deps can do it first.
- Use `cargo clippy -- -D warnings` as the quality gate. Zero warnings.
- Write tests for anything with real logic (path builders, validation, error mapping, business rules)
- Use `tracing` for structured logging (not `println!` or `log`)
- Handle all errors explicitly. No `.unwrap()` in production code (only in tests).

### DO NOT

- Don't over-engineer. No abstractions for one-time operations. Three similar lines > premature abstraction.
- Don't add features that weren't asked for. No speculative generalization.
- Don't hardcode values that should be configurable (URLs, timeouts, limits, names)
- Don't add comments that just restate the code. Only comment WHY, not WHAT.
- Don't create files unless necessary. Prefer editing existing files.
- Don't add dependencies without justification. Every dependency is a maintenance cost.
- Don't use `unsafe` without a comment explaining why it's safe
- Don't ignore compiler warnings. Fix them or explicitly allow with a comment.

### Correctness Over Convenience

These rules override "don't over-engineer" for any code path where data loss, inconsistency, or silent failure has real business impact (billing, state transitions, multi-step operations):

- **Durable writes for critical data.** Never use fire-and-forget (`tokio::spawn`) for events that must survive crashes (billing, audit, state changes). The write must be committed to disk before the handler returns or before losing process control.
- **Errors MUST propagate for critical operations.** If a critical write fails, callers must receive `Result::Err` and decide whether to fail the request. Provide both `_required` (returns Result) and `_best_effort` (logs and swallows) variants where both patterns are needed. Never silently swallow errors on financial or state-changing writes.
- **Related operations MUST be transactional.** When an operation produces both a state change and a side effect (billing, audit, notification), both must commit in the same DB transaction. No crash windows between "operation committed" and "side effect committed."
- **Docs MUST describe actual guarantees, not ideal ones.** If a code path is best-effort, say so. If a subsystem isn't on the durable path yet, say so. Never claim durability, consistency, or coverage you don't have.
- **Streaming/async results use reservation pattern.** When the final value is only known after an async operation (streaming tokens, GPU time), write a durable pending row with conservative fallback BEFORE losing control. Finalize with actual values after completion. Stale pending rows are reaped after timeout.
- **Think through every crash point.** For any multi-step operation, enumerate what happens if the process dies at each step. If any step loses data or creates inconsistency, fix the design — don't document it as a "known limitation."

### Performance Mindset

- This platform will scale to hundreds of instances. Every decision should consider cold start time, memory usage, and throughput.
- Rust is chosen specifically for performance. Don't write Rust that performs like Python.
- Use async everywhere. No blocking operations on the Tokio runtime.
- Use `tokio::try_join!` for parallel independent operations (e.g., count + fetch in list endpoints)
- Streaming over buffering: prefer `Stream` for large file uploads/downloads
- Connection pooling for DB and Redis — never create connections per request

### Clean Code

- One concern per file. If a file has multiple unrelated types, split them.
- Consistent naming: `snake_case` in Rust, `camelCase` in TypeScript, `snake_case` in Python
- Use the type system to make illegal states unrepresentable (enums > booleans)
- Group related items. Organize modules by feature, not by type.

## Testing

### Rust Tests

```bash
cargo test --workspace              # Run all tests
cargo test -p platform-shared       # Test specific crate
cargo test -- test_name             # Test specific function
```

- Unit tests go in the same file as the code (`#[cfg(test)] mod tests`)
- Integration tests go in `tests/` directory within each crate
- Use `#[tokio::test]` for async tests
- Test error cases, not just happy paths
- No tests for trivial code (simple getters, struct construction)

### Frontend Tests

```bash
cd apps/web && pnpm test            # If test runner is set up
cd apps/web && pnpm type-check      # TypeScript as a test
```

## Git Practices

- Commit messages: short imperative ("Add project CRUD", not "Added project CRUD")
- One logical change per commit
- Never commit `.env` files, credentials, or large binaries

## Commands

```bash
make infra          # Start PostgreSQL, Redis, MinIO
make temporal       # Start Temporal server
make migrate        # Run DB migrations
make dev-api        # Start Rust API (hot reload via cargo-watch)
make dev-web        # Start Next.js frontend
make dev-workers    # Start Python Temporal worker
make test           # Run all tests (also regenerates TS types via ts-rs)
make typegen        # Regenerate TypeScript types from Rust (ts-rs)
make lint           # Clippy + rustfmt + frontend lint
make build          # Release build all
```
