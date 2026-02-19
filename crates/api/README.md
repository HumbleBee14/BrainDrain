# crates/api

**The HTTP API server — the single backend entry point for the frontend and all external clients.**

| | |
|---|---|
| Language | Rust |
| Framework | Axum 0.8 + Tokio |
| Auth | Clerk JWT verification (+ dev tokens for local) |
| Deploys as | Single binary in Docker (~20MB image) |
| Port | 8000 (default) |
| Depends on | `crates/shared`, `crates/db`, `crates/storage` |

## Architecture

```
Route Handler (thin) → Service (business logic) → Repository (SQL queries)
```

- **Routes** — extract auth, validate DTOs, return responses
- **Services** — orchestrate repos, S3, Redis, fire events
- **Repositories** — pure SQL via SQLx, every query requires `tenant_id`

## What's Inside

| Folder/File | Purpose |
|---|---|
| `main.rs` | Server startup, middleware stack, graceful shutdown (Ctrl+C / SIGTERM) |
| `config.rs` | Typed config from env vars (`envy` + `dotenvy`) — fails fast on missing config |
| `app_state.rs` | `AppState` — DB pool, Redis connection, S3 client, config |
| `auth.rs` | `AuthenticatedUser` extractor — Clerk JWT in prod, `dev_{tenant}_{user}` in dev |
| `error.rs` | `AppError` enum → `{"error":{"code":"...","message":"..."}}` JSON responses |
| `middleware.rs` | CORS, `X-Request-Id` propagation, structured tracing |
| `routes/` | Health checks, Project CRUD, Document upload |
| `services/` | Business logic layer |
| `repositories/` | SQL query layer (tenant-scoped) |
| `dto/` | Request/response types (serde) |

## Endpoints

| Method | Path | Description |
|---|---|---|
| `GET` | `/health` | Liveness |
| `GET` | `/ready` | Readiness (DB + Redis) |
| `POST` | `/api/v1/projects` | Create project |
| `GET` | `/api/v1/projects` | List (paginated) |
| `GET` | `/api/v1/projects/:id` | Get |
| `PUT` | `/api/v1/projects/:id` | Update |
| `DELETE` | `/api/v1/projects/:id` | Soft delete |
| `POST` | `/api/v1/projects/:id/documents` | Upload (multipart → S3) |
| `GET` | `/api/v1/projects/:id/documents` | List documents |
| `GET` | `/api/v1/documents/:id` | Get document |

## Running Locally

```bash
# Prerequisites: PostgreSQL, Redis, MinIO running (docker compose up -d)
# Copy .env.example to .env and fill in values

make dev-api                       # with cargo-watch (hot reload)
# OR
cargo run --bin platform-api     # direct run
```

### Required Environment Variables

| Variable | Example | Description |
|---|---|---|
| `DATABASE_URL` | `postgresql://platform:platform@localhost:5432/platform` | PostgreSQL connection |
| `REDIS_URL` | `redis://localhost:6379` | Redis connection |
| `S3_ENDPOINT` | `http://localhost:9000` | MinIO/S3 endpoint |
| `S3_ACCESS_KEY` | `minioadmin` | S3 access key |
| `S3_SECRET_KEY` | `minioadmin` | S3 secret key |
| `CLERK_PUBLISHABLE_KEY` | `pk_test_...` | Clerk public key (or use dev tokens) |
| `ENVIRONMENT` | `development` | Enables dev token auth |

### Dev Token Auth (no Clerk needed)

In `development` mode, use `Authorization: Bearer dev_{tenant_uuid}_{user_id}` to skip Clerk JWT verification.

## Docker Build & Deploy

```bash
# Build image (from repo root)
docker build -f crates/api/Dockerfile -t platform-api .

# Run container
docker run -p 8000:8000 --env-file .env platform-api
```

Final image is **~20MB** (Debian slim + static Rust binary).

## Tests

```bash
cargo test -p platform-api
```
