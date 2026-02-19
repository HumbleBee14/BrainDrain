# Phase 0 — Foundation Build (Complete)

> Infrastructure skeleton that all future ML engineering (training, data refinery, evaluation) is built on top of.

## What Was Built

Phase 0 delivers a fully wired monorepo with 3 independently deployable services, a shared type system, local infrastructure via Docker, CI/CD, and comprehensive tests. No ML code yet — this is pure infrastructure.

---

## Project Structure

```
BrainDrain/
├── Cargo.toml                          # Rust workspace root
├── Cargo.lock
├── package.json                        # JS workspace root (pnpm)
├── pnpm-workspace.yaml                 # ["apps/web", "packages/*"]
├── pnpm-lock.yaml                      # Locked JS dependencies
├── turbo.json                          # Turborepo (frontend builds only)
├── Makefile                            # make infra, make dev-api, make test, etc.
├── docker-compose.yml                  # PostgreSQL 16, Redis 7, MinIO
├── .env.example                        # All env vars template
├── .gitignore
├── CLAUDE.md                           # AI assistant context + dev guidelines
├── README.md                           # Project intro + quick start
│
├── docs/
│   ├── ARCHITECTURE.md                 # System architecture, decision records
│   ├── RESEARCH.md                     # LLM fine-tuning landscape analysis
│   ├── DEVELOPMENT.md                  # Dev tracker, phases overview
│   └── phase0/
│       └── PHASE0_COMPLETE.md          # This file
│
├── .github/workflows/
│   ├── ci.yml                          # 3-job CI: Rust + Frontend + Python
│   └── deploy-staging.yml              # Docker image builds for staging
│
├── crates/                             # ── Rust backend ──
│   ├── shared/                         # Shared types, enums, constants
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── enums.rs                # 13 enums (DocumentStatus, TrainingJobStatus, etc.)
│   │       ├── constants.rs            # Upload limits, Redis keys, Temporal queues
│   │       ├── s3_paths.rs             # Tenant-scoped S3 path builders
│   │       └── events.rs              # 7 pipeline event structs
│   │
│   ├── db/                             # Database models + migrations
│   │   └── src/
│   │       ├── lib.rs                  # create_pool(), re-exports
│   │       ├── models.rs               # SQLx FromRow structs (9 tables)
│   │       ├── migrate.rs              # Migration runner binary
│   │       └── migrations/
│   │           └── 001_initial_schema.sql  # Full 9-table schema + RLS + triggers
│   │
│   ├── storage/                        # S3/R2/MinIO client
│   │   └── src/
│   │       ├── lib.rs                  # ObjectStorage trait + StorageError
│   │       └── s3.rs                   # S3Storage impl (put, get, delete, presign, exists)
│   │
│   └── api/                            # HTTP API server (Axum)
│       ├── Dockerfile                  # Multi-stage Rust build (~20MB final)
│       └── src/
│           ├── main.rs                 # Server startup, router, graceful shutdown
│           ├── config.rs               # Env-based config via envy + dotenvy
│           ├── app_state.rs            # Shared state: DB pool, Redis, S3 client
│           ├── error.rs                # AppError enum → HTTP status + JSON envelope
│           ├── auth.rs                 # Clerk JWT verification + dev token support
│           ├── middleware.rs            # CORS, request ID (X-Request-Id), tracing
│           ├── routes/
│           │   ├── mod.rs              # /api/v1 router + health merge
│           │   ├── health.rs           # GET /health, GET /ready
│           │   ├── projects.rs         # CRUD /api/v1/projects
│           │   └── documents.rs        # Upload + list /api/v1/projects/:id/documents
│           ├── services/
│           │   ├── project_service.rs  # Project business logic
│           │   └── document_service.rs # Upload validation, S3 store, DB record
│           ├── repositories/
│           │   ├── project_repo.rs     # Project SQL queries (tenant-scoped)
│           │   └── document_repo.rs    # Document SQL queries (tenant-scoped)
│           └── dto/
│               ├── common.rs           # PaginatedResponse, ErrorEnvelope
│               ├── project.rs          # CreateProject, ProjectResponse
│               └── document.rs         # UploadResponse, DocumentResponse
│
├── apps/
│   ├── web/                            # ── Next.js 15 frontend ──
│   │   ├── Dockerfile                  # 3-stage Node build (standalone)
│   │   ├── package.json
│   │   ├── eslint.config.mjs           # ESLint 9 flat config
│   │   ├── tailwind.config.ts
│   │   ├── next.config.ts
│   │   ├── .env.local.example
│   │   └── src/
│   │       ├── app/
│   │       │   ├── layout.tsx          # Root: ClerkProvider + Providers wrapper
│   │       │   ├── page.tsx            # Landing page
│   │       │   ├── globals.css         # Tailwind + shadcn CSS vars
│   │       │   ├── providers.tsx       # React Query provider
│   │       │   ├── (auth)/
│   │       │   │   ├── sign-in/[[...sign-in]]/page.tsx
│   │       │   │   └── sign-up/[[...sign-up]]/page.tsx
│   │       │   └── (dashboard)/
│   │       │       ├── layout.tsx      # Sidebar + header layout
│   │       │       ├── dashboard/page.tsx
│   │       │       ├── projects/page.tsx
│   │       │       ├── projects/new/page.tsx
│   │       │       └── projects/[id]/page.tsx
│   │       ├── lib/
│   │       │   ├── api-client.ts       # Typed fetch wrapper with auth token
│   │       │   └── utils.ts            # cn() helper
│   │       ├── hooks/
│   │       │   └── use-projects.ts     # React Query hooks for projects CRUD
│   │       └── middleware.ts           # Clerk auth route protection
│   │
│   └── workers/                        # ── Python Temporal workers ──
│       ├── Dockerfile                  # 2-stage Python build with uv
│       ├── pyproject.toml              # uv-managed, ruff dev dep, optional [ml] extras
│       └── src/
│           ├── __init__.py
│           ├── config.py               # Pydantic Settings (APP_ prefix)
│           ├── worker.py               # Temporal worker entrypoint
│           ├── activities/
│           │   └── stubs.py            # 6 typed activity stubs with dataclass I/O
│           └── workflows/
│               ├── ingest.py           # Upload → Parse documents
│               ├── refine.py           # Parsed docs → Synthetic pairs
│               ├── train.py            # Dataset → LoRA adapter
│               ├── evaluate.py         # Model → Scores + report
│               └── full_pipeline.py    # Chains all stages
│
├── packages/
│   └── shared-types/                   # ── TypeScript API types ──
│       ├── package.json
│       ├── tsconfig.json
│       └── src/
│           ├── index.ts
│           ├── api.ts                  # Response types mirroring Rust DTOs
│           └── enums.ts                # Status enums mirroring Rust enums
│
└── infra/
    ├── temporal/
    │   └── docker-compose.temporal.yml # Temporal server + UI
    └── scripts/
        └── init-db.sh                  # DB initialization script
```

**File count**: ~80+ source files across 3 languages (Rust, TypeScript, Python).

---

## Language Split

| Layer | Language | Framework | Why |
|---|---|---|---|
| API Gateway | Rust | Axum 0.8 | Fastest HTTP, minimal memory, instant cold starts |
| Database layer | Rust | SQLx 0.8 | Compile-time SQL checks, async, connection pooling |
| Object storage | Rust | aws-sdk-s3 | Official AWS SDK, streaming support |
| Auth (JWT) | Rust | jsonwebtoken | JWT verification, JWKS fetching |
| Redis/cache | Rust | redis-rs 0.27 | Async, connection pooling |
| ML training | Python | Temporal SDK | ML ecosystem is Python-only (Unsloth, TRL, distilabel) |
| Workflow orchestration | Python | Temporal SDK | Workers call Python ML libraries |
| Frontend | TypeScript | Next.js 15, React 19 | React ecosystem, Clerk auth |

**Key principle**: Rust for all infrastructure. Python only for ML-specific code. TypeScript only for frontend.

---

## Database Schema (9 Tables)

Built in `crates/db/src/migrations/001_initial_schema.sql`:

| Table | Purpose | Key Columns |
|---|---|---|
| `tenants` | Maps to Clerk organizations | clerk_org_id, plan, settings (JSONB) |
| `projects` | User's fine-tuning projects | tenant_id, name, task_type, status, config (JSONB) |
| `documents` | Uploaded files for training | project_id, filename, storage_path, parse_quality |
| `datasets` | Generated training data | project_id, format, pair_count, stats (JSONB) |
| `training_jobs` | Fine-tuning runs | dataset_id, base_model, method, hyperparams (JSONB), gpu_class |
| `models` | Trained LoRA adapters | training_job_id, adapter_path, deployment_status, eval_scores (JSONB) |
| `evaluations` | Model quality assessments | model_id, scores (JSONB), report (JSONB) |
| `api_keys` | Inference access keys | model_id, key_hash, rate_limit, is_active |
| `billing_events` | Usage ledger (append-only) | operation, tokens_in/out, gpu_seconds, cost_usd |

**Multi-tenancy enforcement**:
- Every table has `tenant_id` FK + index
- Row-Level Security (RLS) enabled on all tenant-scoped tables
- Repository layer requires `tenant_id` parameter on every query
- S3 paths scoped: `{tenant_id}/{project_id}/...`

**Indexes**: Composite on `(tenant_id, status)` and `(tenant_id, created_at)` for efficient filtered queries.

**Triggers**: `updated_at` auto-update trigger on all tables with that column.

---

## API Endpoints

| Method | Path | Handler | Status |
|---|---|---|---|
| `GET` | `/health` | Health check | Working |
| `GET` | `/ready` | Readiness probe (DB ping) | Working |
| `POST` | `/api/v1/projects` | Create project | Working |
| `GET` | `/api/v1/projects` | List projects (paginated) | Working |
| `GET` | `/api/v1/projects/:id` | Get project | Working |
| `DELETE` | `/api/v1/projects/:id` | Soft-delete project | Working |
| `POST` | `/api/v1/projects/:id/documents` | Upload document (multipart → S3) | Working |
| `GET` | `/api/v1/projects/:id/documents` | List documents (paginated) | Working |

**Architecture pattern**: Route handler (thin) → Service (business logic) → Repository (SQL, tenant_id required)

**Error handling**: `AppError` enum → `IntoResponse` → consistent JSON envelope:
```json
{"error": {"code": "NOT_FOUND", "message": "Project not found"}}
```

Internal errors (DB, storage, anyhow) return generic message to avoid leaking details.

**Middleware stack**:
- `X-Request-Id` generation and propagation
- CORS (configurable origins)
- HTTP tracing (structured logging)
- JSON logging in production, pretty logging in dev

**Auth**: Clerk JWT verification via JWKS endpoint. Dev mode supports `dev_{tenant_uuid}_{user_id}` tokens for local testing without Clerk.

---

## Frontend Pages

| Page | Route | Features |
|---|---|---|
| Landing | `/` | Hero section, feature highlights, CTA |
| Sign In | `/sign-in` | Clerk-hosted auth UI |
| Sign Up | `/sign-up` | Clerk-hosted auth UI |
| Dashboard | `/dashboard` | Overview stats, recent projects |
| Project List | `/projects` | Cards with status badges, create button |
| New Project | `/projects/new` | Form: name, description, task type selector |
| Project Detail | `/projects/[id]` | Info grid, document upload area, pipeline stages, delete |

**Data fetching**: React Query hooks (`useProjects`, `useProject`, `useCreateProject`, `useDeleteProject`) with Clerk token injection via `api-client.ts`.

**UI**: Tailwind CSS, dark theme (zinc-950 base), shadcn/ui component patterns.

**Brand independence**: App name comes from `NEXT_PUBLIC_APP_NAME` env var, defaults to "Platform".

---

## Temporal Workflows (Stubs)

All workflow and activity files are scaffolded with correct signatures, typed I/O (dataclasses), and proper timeouts. Implementation is Phase 1+.

| Workflow | Stages | Timeout |
|---|---|---|
| `IngestWorkflow` | parse_document | 10 min/doc |
| `RefineWorkflow` | generate_synthetic_pairs | 30 min |
| `TrainWorkflow` | build_dataset → start_training | 6 hours |
| `EvaluateWorkflow` | run_evaluation | 1 hour |
| `FullPipelineWorkflow` | Chains all of the above | Sum of above |

**Activities** (6 total): `parse_document`, `generate_synthetic_pairs`, `build_dataset`, `start_training`, `run_evaluation`, `deploy_model`. Each has typed `*Input` and `*Output` dataclasses.

---

## Infrastructure (Docker Compose)

| Service | Image | Port | Purpose |
|---|---|---|---|
| PostgreSQL 16 | `postgres:16-alpine` | 5432 | Primary database |
| Redis 7 | `redis:7-alpine` | 6379 | Caching, rate limiting, job status |
| MinIO | `minio/minio:latest` | 9000/9001 | S3-compatible object storage |
| minio-init | `minio/mc` | — | Creates default bucket on startup |

Separate `infra/temporal/docker-compose.temporal.yml` for Temporal server + UI (heavyweight, optional).

---

## CI/CD Pipeline

`.github/workflows/ci.yml` — 3 parallel jobs on every PR and push to main:

| Job | Steps | What It Checks |
|---|---|---|
| **Rust** | `cargo fmt --check` → `cargo clippy -Dwarnings` → `cargo test` | Formatting, lints (zero warnings), 20 unit tests |
| **Frontend** | `pnpm install --frozen-lockfile` → `eslint src/` → `tsc --noEmit` | Lint rules, type safety for web + shared-types |
| **Python** | `uv sync` → `ruff check src/` → `ruff format --check src/` | Lint (E/F/I/UP rules), import ordering, formatting |

Rust job spins up a PostgreSQL service container for DB-dependent tests.

`.github/workflows/deploy-staging.yml` — Builds Docker images for all 3 services.

---

## Tests (20 total)

| Crate/File | Tests | What's Covered |
|---|---|---|
| `shared/enums.rs` | 6 | Serialization to/from snake_case, Display, FromStr, roundtrip |
| `shared/events.rs` | 3 | Event type strings, JSON serialization, roundtrip |
| `shared/constants.rs` | 4 | Upload limit, supported extensions, API key prefix, queue names |
| `shared/s3_paths.rs` | 2 | Upload path format, adapter prefix |
| `api/error.rs` | 5 | Status codes, error codes, internal leak prevention, envelope shape |

All 20 pass with `RUSTFLAGS="-Dwarnings"` (zero warnings mode).

---

## Dockerfiles

| Service | Base | Final Size | Build Strategy |
|---|---|---|---|
| API (Rust) | `debian:bookworm-slim` | ~20MB | Multi-stage: dependency cache layer → build → copy binary only |
| Web (Next.js) | `node:20-alpine` | ~100MB | 3-stage: deps → build → standalone output |
| Workers (Python) | `python:3.11-slim` | ~500MB+ | 2-stage: uv dependency resolution → slim runtime |

---

## Key Architectural Decisions

1. **Monorepo, not monolith** — Single Git repo, but each service (api, web, workers) deploys independently in its own container. Zero runtime coupling.

2. **Brand independence** — No brand names anywhere in code. Generic naming (`platform-*`), brand only via env vars (`NEXT_PUBLIC_APP_NAME`).

3. **Trait abstractions** — `ObjectStorage` trait means S3 backend is swappable (AWS S3, R2, MinIO) without changing any service code.

4. **Fail-fast config** — All env vars deserialized into typed Rust struct at startup. Missing required vars crash immediately, not at request time.

5. **Repository pattern with tenant enforcement** — Every DB query goes through a repo function that requires `tenant_id`. Impossible to accidentally query across tenants.

6. **Event-driven extensibility** — 7 pipeline event structs ready for message bus (Redis streams or similar) when needed.

---

## Build Steps Completed

| # | Step | Status |
|---|---|---|
| 1 | Root scaffolding (Cargo.toml, package.json, Makefile, etc.) | Done |
| 2 | Docker Compose (PostgreSQL, Redis, MinIO, Temporal) | Done |
| 3 | `crates/shared` — Enums, constants, events, S3 paths | Done |
| 4 | `crates/db` — Models, migrations, 9-table schema | Done |
| 5 | `crates/storage` — S3 client with ObjectStorage trait | Done |
| 6 | `crates/api` — Config, app state, error handling, auth, middleware | Done |
| 7 | `crates/api` — Routes, services, repositories (project CRUD, doc upload) | Done |
| 8 | `apps/web` — Next.js frontend (auth, dashboard, project pages, React Query) | Done |
| 9 | `apps/workers` — Python Temporal workers (5 workflow stubs, 6 activity stubs) | Done |
| 10 | `packages/shared-types` — TypeScript API types mirroring Rust DTOs | Done |
| 11 | CI/CD — GitHub Actions (Rust + Frontend + Python, 3 parallel jobs) | Done |
| 12 | Dockerfiles — Multi-stage builds for all 3 services | Done |

---

## What's Next — Phase 1 (Data Pipeline)

Phase 1 is where real user-facing functionality begins: upload a document, parse it, and generate training data.

**Key deliverables**:
- Document parsing (PDF, DOCX, HTML, images via OCR) using MinerU/python-docx
- Text chunking with configurable token sizes
- Synthetic instruction/response pair generation using distilabel + LLM API
- Quality scoring and filtering of generated pairs
- Dataset assembly in ChatML/ShareGPT format
- Temporal IngestWorkflow and RefineWorkflow fully implemented
- Real-time progress tracking via Redis streams
- Frontend: document upload with progress, parsed content preview, dataset review UI

**Infrastructure needed from Phase 0 that Phase 1 builds on**:
- S3 streaming upload (documents) → already wired
- Database records for documents + datasets → schema ready
- Temporal workflow stubs → signatures ready, fill in ML code
- API routes for upload + list → already working
- React Query hooks → already fetching from API
