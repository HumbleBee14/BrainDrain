# Platform — Development Tracker

> Fine-tune LLMs on your data — no technical knowledge required.

---

## Table of Contents

- [Project Structure](#project-structure)
- [Language Split](#language-split)
- [Architecture Pattern](#architecture-pattern)
- [Deployment Model](#deployment-model)
- [Phase Status](#phase-status)
- [Phase 0: Foundation (COMPLETE)](#phase-0-foundation--complete)
- [Phase 1: Data Pipeline (NEXT)](#phase-1-data-pipeline--next)
- [Phase 2: Training Engine](#phase-2-training-engine)
- [Phase 3: Evaluation System](#phase-3-evaluation-system)
- [Phase 4: Model Deployment](#phase-4-model-deployment)
- [Phase 5: Product Polish](#phase-5-product-polish)
- [Local Development](#local-development)

---

## Project Structure

```
Platform/
│
├── docs/                                # ═══ DOCUMENTATION ═══
│   ├── ARCHITECTURE.md                  #   System architecture, TDRs, component registry
│   ├── RESEARCH.md                      #   LLM fine-tuning landscape, competitors, GPU infra
│   └── DEVELOPMENT.md                   #   This file — development tracker
│
├── Cargo.toml                           # Rust workspace root (all dep versions centralized)
├── package.json                         # JS workspace root (pnpm)
├── pnpm-workspace.yaml                  # Workspace: ["apps/web", "packages/*"]
├── turbo.json                           # Turborepo pipeline (frontend only)
├── Makefile                             # make dev-api, make migrate, make test, make lint
├── docker-compose.yml                   # PostgreSQL 16, Redis 7, MinIO (local infra)
├── .env.example                         # All environment variables template
│
├── crates/                              # ═══ RUST BACKEND (Performance-critical infra) ═══
│   │
│   ├── shared/                          # Shared types across all Rust crates
│   │   └── src/
│   │       ├── enums.rs                 #   DocumentStatus, TrainingJobStatus, PipelineStage, etc.
│   │       ├── constants.rs             #   Temporal queues, Redis keys, upload limits
│   │       ├── s3_paths.rs              #   Tenant-scoped S3 path builders (with tests)
│   │       └── events.rs               #   Pipeline event structs for message bus
│   │
│   ├── db/                              # Database layer (SQLx + PostgreSQL)
│   │   └── src/
│   │       ├── models.rs                #   9 SQLx FromRow structs (Tenant → BillingEvent)
│   │       ├── migrations/
│   │       │   └── 001_initial_schema.sql  # Full schema: 9 tables, indexes, RLS, triggers
│   │       ├── lib.rs                   #   create_pool(), run_migrations()
│   │       └── migrate.rs              #   Standalone migration binary
│   │
│   ├── storage/                         # S3/object storage abstraction
│   │   └── src/
│   │       ├── lib.rs                   #   ObjectStorage TRAIT (put/get/exists/delete/presign)
│   │       └── s3.rs                    #   S3Storage impl (works with AWS S3/R2/MinIO)
│   │
│   └── api/                             # HTTP API server (Axum)
│       ├── Dockerfile                   #   Multi-stage Rust build (~20MB final image)
│       └── src/
│           ├── main.rs                  #   Server startup, middleware stack, graceful shutdown
│           ├── config.rs                #   Typed env config (envy + dotenvy)
│           ├── app_state.rs             #   AppState: DB pool + Redis + S3 + Config
│           ├── error.rs                 #   AppError → JSON error envelope {"error":{...}}
│           ├── auth.rs                  #   Clerk JWT verification + dev token support
│           ├── middleware.rs            #   CORS, request ID (X-Request-Id), tracing
│           ├── routes/
│           │   ├── health.rs            #     GET /health (liveness), GET /ready (readiness)
│           │   ├── projects.rs          #     CRUD: POST/GET/PUT/DELETE /api/v1/projects
│           │   └── documents.rs         #     Multipart upload: POST /api/v1/projects/:id/documents
│           ├── services/
│           │   ├── project_service.rs   #     Business logic (validation, orchestration)
│           │   └── document_service.rs  #     Upload → S3 → DB, uses ObjectStorage trait
│           ├── repositories/
│           │   ├── project_repo.rs      #     SQL queries (ALL require tenant_id — enforced)
│           │   └── document_repo.rs     #     SQL queries (ALL require tenant_id — enforced)
│           └── dto/
│               ├── common.rs            #     PaginationParams, PaginatedResponse<T>
│               ├── project.rs           #     CreateProject, UpdateProject, ProjectResponse
│               └── document.rs          #     UploadResponse, DocumentResponse
│
├── apps/                                # ═══ APPLICATIONS ═══
│   │
│   ├── web/                             # Next.js 15 frontend (TypeScript)
│   │   ├── Dockerfile                   #   3-stage build (deps → build → standalone)
│   │   └── src/
│   │       ├── middleware.ts            #   Clerk auth middleware (protects /dashboard, /projects)
│   │       ├── app/
│   │       │   ├── layout.tsx           #     Root: ClerkProvider + QueryClientProvider
│   │       │   ├── providers.tsx        #     React Query setup (30s stale time)
│   │       │   ├── page.tsx             #     Landing page
│   │       │   ├── (auth)/
│   │       │   │   ├── sign-in/         #     Clerk SignIn component
│   │       │   │   └── sign-up/         #     Clerk SignUp component
│   │       │   └── (dashboard)/
│   │       │       ├── layout.tsx       #     Sidebar + header layout
│   │       │       ├── dashboard/       #     Stats overview (projects, models, docs counters)
│   │       │       └── projects/
│   │       │           ├── page.tsx     #       Project list (loading/empty/data states)
│   │       │           ├── new/         #       Create project form (name, description, task type)
│   │       │           └── [id]/        #       Project detail (info, doc upload, pipeline, delete)
│   │       ├── hooks/
│   │       │   └── use-projects.ts      #   React Query hooks (list, get, create, delete)
│   │       └── lib/
│   │           ├── api-client.ts        #   Typed fetch wrapper with Clerk token injection
│   │           └── utils.ts             #   cn() helper (clsx + tailwind-merge)
│   │
│   └── workers/                         # Python Temporal ML workers
│       ├── Dockerfile                   #   2-stage (uv build → python:3.11-slim runtime)
│       ├── pyproject.toml               #   temporalio, pydantic-settings; ML deps as [ml] extras
│       └── src/
│           ├── config.py                #   Worker settings via pydantic-settings
│           ├── worker.py                #   Temporal worker entrypoint (registers all workflows)
│           ├── activities/
│           │   └── stubs.py             #   6 typed activities with dataclass I/O:
│           │                            #     parse_document, generate_synthetic_pairs,
│           │                            #     build_dataset, start_training, run_evaluation,
│           │                            #     deploy_model
│           └── workflows/
│               ├── ingest.py            #   Upload → Parse documents
│               ├── refine.py            #   Parsed docs → Synthetic instruction pairs
│               ├── train.py             #   Dataset → Fine-tuned LoRA adapter
│               ├── evaluate.py          #   Model → Scores + evaluation report
│               └── full_pipeline.py     #   Chains ALL 6 stages end-to-end
│
├── packages/                            # ═══ SHARED PACKAGES ═══
│   └── shared-types/                    # TypeScript API types (mirrors Rust DTOs)
│       └── src/
│           ├── enums.ts                 #   All status enums (mirrors crates/shared/enums.rs)
│           ├── api.ts                   #   Response types: Project, Document, Dataset, Model, etc.
│           └── index.ts                 #   Re-exports
│
├── infra/                               # ═══ INFRASTRUCTURE ═══
│   ├── temporal/
│   │   └── docker-compose.temporal.yml  #   Temporal server + PostgreSQL + UI (port 8088)
│   └── scripts/
│       └── init-db.sh                   #   Database initialization script
│
└── .github/workflows/                   # ═══ CI/CD ═══
    ├── ci.yml                           #   3 parallel jobs: Rust (fmt+clippy+test),
    │                                    #     Frontend (lint+typecheck), Python (ruff)
    └── deploy-staging.yml               #   Docker image builds for all 3 services
```

---

## Language Split

| Layer | Language | Why |
|---|---|---|
| API Gateway | **Rust (Axum)** | Fastest HTTP framework, ~5MB binary, instant cold starts |
| File upload/streaming | **Rust** | Zero-copy streaming to S3, backpressure handling |
| Database layer | **Rust (SQLx)** | Compile-time checked SQL, async connection pooling |
| Redis/cache | **Rust (redis-rs)** | Async, multiplexed connections |
| S3/storage | **Rust (aws-sdk-rust)** | Official AWS SDK, streaming support |
| Auth (Clerk JWT) | **Rust (jsonwebtoken)** | JWT verification, JWKS fetching |
| ML training | **Python (Unsloth/TRL)** | ML ecosystem is Python-only |
| Synthetic data gen | **Python (distilabel)** | ML pipeline, LLM API calls |
| Document parsing | **Python (MinerU)** | PDF/DOCX parsing libraries |
| Workflow orchestration | **Python (Temporal SDK)** | ML workers are Python anyway |
| Frontend | **TypeScript (Next.js 15)** | React ecosystem |

**Rule:** Rust for all infrastructure. Python only where ML libraries force it.

---

## Architecture Pattern

```
┌─────────────────────────────────────────────────────────┐
│                    Rust API (Axum)                       │
│                                                         │
│  Route Handler (thin)                                   │
│      │  - Extract auth (Clerk JWT)                      │
│      │  - Validate DTO                                  │
│      │  - Return response DTO                           │
│      ▼                                                  │
│  Service Layer (business logic)                         │
│      │  - Orchestrates repos, S3, Redis, events         │
│      │  - Validation rules                              │
│      ▼                                                  │
│  Repository Layer (data access)                         │
│      │  - Pure SQL queries via SQLx                     │
│      │  - EVERY query requires tenant_id (enforced)     │
│      ▼                                                  │
│  PostgreSQL (multi-tenant, RLS enabled)                 │
└─────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────┐
│               Python Workers (Temporal)                  │
│                                                         │
│  Workflow (orchestration)                               │
│      │  - Defines stage order and retry logic           │
│      │  - Visible in Temporal UI for observability      │
│      ▼                                                  │
│  Activity (execution unit)                              │
│      │  - Idempotent, individually retryable            │
│      │  - Does the actual ML work                       │
│      ▼                                                  │
│  ML Libraries (Unsloth, distilabel, MinerU, vLLM)      │
└─────────────────────────────────────────────────────────┘
```

### Key Design Decisions

- **ObjectStorage trait** — S3 backend is swappable (AWS S3, Cloudflare R2, MinIO, or any future provider)
- **Multi-tenancy at repo layer** — impossible to accidentally query another tenant's data
- **Dev token auth** — `dev_{tenant_uuid}_{user_id}` format for local development without Clerk
- **Temporal activity stubs** — typed input/output dataclasses ready for ML code to fill in
- **Soft deletes** — projects use `deleted_at` instead of hard delete

---

## Deployment Model

**This is a monorepo, NOT a monolith.** Each component deploys independently:

| Component | Container | Size | Scales... |
|---|---|---|---|
| `crates/api/` | `platform-api` | ~20MB | Horizontally (stateless, any cloud) |
| `apps/web/` | `platform-web` | ~50MB | Horizontally (stateless, Vercel/any cloud) |
| `apps/workers/` | `platform-workers` | ~500MB+ | Vertically (GPU machines) |

Each has its own `Dockerfile`. They share **zero runtime dependencies** — communicate only via HTTP API and Temporal task queues.

---

## Phase Status

| Phase | Status | Focus |
|---|---|---|
| **Phase 0: Foundation** | **COMPLETE** | Infrastructure skeleton — everything below |
| **Phase 1: Data Pipeline** | **NEXT** | Document parsing, synthetic data generation, dataset building |
| **Phase 2: Training Engine** | PENDING | Unsloth/TRL fine-tuning, GPU orchestration |
| **Phase 3: Evaluation** | PENDING | LLM-as-judge, task-specific metrics, reporting |
| **Phase 4: Deployment** | PENDING | vLLM inference, LoRA adapter serving, API keys |
| **Phase 5: Product Polish** | PENDING | Billing, team management, onboarding, dashboards |

---

## Phase 0: Foundation — COMPLETE

Everything that supports the ML engineering work. "Product building" stuff that had to exist first.

### What Was Built

| Step | Component | What It Does |
|---|---|---|
| Research | `docs/RESEARCH.md` | LLM fine-tuning landscape 2025-26, competitor analysis, GPU infra research, data curation challenges |
| Architecture | `docs/ARCHITECTURE.md` | 12 TDRs, Rust vs Go vs Python comparison tables, component registry, data flow diagrams, database schema |
| Root scaffolding | `Cargo.toml`, `package.json`, `Makefile` | Rust workspace + JS workspace + developer commands |
| Docker Compose | `docker-compose.yml` | PostgreSQL 16, Redis 7, MinIO with auto-bucket creation |
| Temporal | `infra/temporal/` | Temporal server + PostgreSQL + Web UI on port 8088 |
| Shared crate | `crates/shared/` | Enums (13 types), constants, S3 path builders (with tests), pipeline events |
| Database | `crates/db/` | 9 SQLx models, full SQL migration (9 tables, 20+ indexes, RLS, triggers) |
| Storage | `crates/storage/` | `ObjectStorage` trait + S3 implementation (AWS/R2/MinIO compatible) |
| API core | `crates/api/` (infra) | Config, AppState, error handling, Clerk JWT auth, middleware (CORS, request ID, tracing) |
| API routes | `crates/api/` (routes) | Health checks, Project CRUD, Document multipart upload — all via 3-layer pattern |
| Frontend | `apps/web/` | Landing page, Clerk auth (sign-in/sign-up), dashboard, project CRUD pages, React Query hooks |
| Workers | `apps/workers/` | 5 Temporal workflows + 6 activity stubs with typed I/O (ready for ML code) |
| Shared types | `packages/shared-types/` | TypeScript enums + API response types mirroring Rust DTOs |
| CI/CD | `.github/workflows/` | Rust (fmt+clippy+test), Frontend (lint+typecheck), Python (ruff) |
| Dockerfiles | 3x `Dockerfile` | Multi-stage builds: API (~20MB), Web (standalone), Workers (python-slim) |

### Database Schema (9 tables)

| Table | Purpose |
|---|---|
| `tenants` | Organizations (linked to Clerk org) |
| `projects` | User projects (group docs + models) |
| `documents` | Uploaded files (PDF, DOCX, etc.) |
| `datasets` | Training-ready datasets (built from refined pairs) |
| `training_jobs` | Fine-tuning jobs (tracks GPU time, cost, metrics) |
| `models` | Trained model artifacts (LoRA adapters) |
| `evaluations` | Model evaluation results (scores, reports) |
| `api_keys` | Inference API keys (per-model, rate-limited) |
| `billing_events` | Usage tracking (tokens, GPU seconds, cost) |

### API Endpoints (implemented)

| Method | Path | Description |
|---|---|---|
| `GET` | `/health` | Liveness check |
| `GET` | `/ready` | Readiness check (DB + Redis) |
| `POST` | `/api/v1/projects` | Create project |
| `GET` | `/api/v1/projects` | List projects (paginated) |
| `GET` | `/api/v1/projects/:id` | Get project |
| `PUT` | `/api/v1/projects/:id` | Update project |
| `DELETE` | `/api/v1/projects/:id` | Soft-delete project |
| `POST` | `/api/v1/projects/:id/documents` | Upload document (multipart → S3) |
| `GET` | `/api/v1/projects/:id/documents` | List documents |
| `GET` | `/api/v1/documents/:id` | Get document |

### Temporal Workflows (stubbed)

| Workflow | Stages | Timeout |
|---|---|---|
| `IngestWorkflow` | parse_document (per doc) | 10 min/doc |
| `RefineWorkflow` | generate_synthetic_pairs | 30 min |
| `TrainWorkflow` | start_training | 6 hours |
| `EvaluateWorkflow` | run_evaluation | 1 hour |
| `FullPipelineWorkflow` | All 6 stages chained | Sum of above |

### Verification Checklist

- [x] `cargo check` — Rust workspace compiles (only dead_code warnings for scaffolded stubs)
- [x] All 78 source files created across Rust, TypeScript, Python
- [x] 3 Dockerfiles build independently
- [x] CI pipeline covers all 3 languages
- [x] Every DB query enforces `tenant_id`
- [x] ObjectStorage trait allows swapping S3 backend
- [x] Dev token auth works without Clerk for local development

---

## Phase 1: Data Pipeline — NEXT

> Fill in the Temporal activity stubs with actual ML code.

### Goals

- Parse uploaded documents into structured text
- Generate high-quality synthetic instruction/response pairs
- Build training-ready datasets with proper formatting

### Work Items

| Task | File to Modify | ML Library | Status |
|---|---|---|---|
| PDF parsing | `activities/stubs.py` → `parse_document` | MinerU | PENDING |
| DOCX parsing | `activities/stubs.py` → `parse_document` | python-docx | PENDING |
| Text chunking + quality scoring | `activities/stubs.py` → `parse_document` | custom | PENDING |
| Synthetic pair generation | `activities/stubs.py` → `generate_synthetic_pairs` | distilabel | PENDING |
| LLM-as-judge filtering | `activities/stubs.py` → `generate_synthetic_pairs` | distilabel | PENDING |
| Dataset formatting (chat template) | `activities/stubs.py` → `build_dataset` | HuggingFace datasets | PENDING |
| Train/val split | `activities/stubs.py` → `build_dataset` | HuggingFace datasets | PENDING |
| Frontend: document status tracking | `apps/web/` | — | PENDING |
| Frontend: dataset preview UI | `apps/web/` | — | PENDING |
| API: dataset endpoints | `crates/api/` | — | PENDING |

---

## Phase 2: Training Engine

### Goals

- Fine-tune models using Unsloth (4x faster LoRA/QLoRA)
- Support multiple training methods (SFT, DPO, ORPO)
- Track metrics, costs, and GPU usage

### Work Items

| Task | Description | Status |
|---|---|---|
| Unsloth integration | `start_training` activity with FastModel | PENDING |
| TRL SFTTrainer setup | Standard HuggingFace training loop | PENDING |
| Hyperparameter management | Map user config → trainer args | PENDING |
| Checkpoint management | Save/resume from S3 | PENDING |
| Metrics streaming | Real-time loss/accuracy to frontend via Redis | PENDING |
| GPU class selection | Match job requirements to GPU tier | PENDING |
| Cost estimation | Predict cost before training starts | PENDING |
| Frontend: training dashboard | Live metrics, progress, cost tracking | PENDING |

---

## Phase 3: Evaluation System

### Goals

- Automatically evaluate fine-tuned models
- LLM-as-judge for quality assessment
- Task-specific metrics (BLEU, ROUGE, accuracy, etc.)

### Work Items

| Task | Description | Status |
|---|---|---|
| Evaluation harness | `run_evaluation` activity with eval suite | PENDING |
| LLM-as-judge prompts | Quality scoring via frontier model | PENDING |
| Task-specific metrics | BLEU/ROUGE for summarization, accuracy for classification | PENDING |
| Report generation | Human-readable eval report (strengths, weaknesses) | PENDING |
| A/B comparison | Compare base model vs fine-tuned | PENDING |
| Frontend: eval results UI | Scores, charts, comparison view | PENDING |

---

## Phase 4: Model Deployment

### Goals

- Serve fine-tuned models via vLLM
- LoRA adapter hot-loading (no full model copies)
- API key management with rate limiting

### Work Items

| Task | Description | Status |
|---|---|---|
| vLLM integration | `deploy_model` activity with vLLM server | PENDING |
| LoRA adapter loading | Hot-swap adapters on base model | PENDING |
| Inference endpoint | REST API for model queries | PENDING |
| API key management | Generate, rotate, revoke keys | PENDING |
| Rate limiting | Per-key RPM limits via Redis | PENDING |
| Usage metering | Token counting, billing events | PENDING |
| Frontend: playground | Test your model in-browser | PENDING |
| Frontend: API docs | Auto-generated endpoint docs | PENDING |

---

## Phase 5: Product Polish

### Goals

- Production-ready billing, team management, onboarding

### Work Items

| Task | Description | Status |
|---|---|---|
| Stripe billing integration | Usage-based pricing | PENDING |
| Team management | Invite members, roles (admin/member/viewer) | PENDING |
| Onboarding flow | Guided first-project experience | PENDING |
| Usage dashboard | Costs, API calls, storage breakdown | PENDING |
| Notifications | Email/webhook on training complete, errors | PENDING |
| Audit log | Track who did what | PENDING |

---

## Local Development

```bash
# 1. Start infrastructure
docker compose up -d                        # PostgreSQL, Redis, MinIO
docker compose -f infra/temporal/docker-compose.temporal.yml up -d  # Temporal

# 2. Run database migrations
make migrate

# 3. Start API server (Rust)
make dev-api

# 4. Start frontend (Next.js)
cd apps/web && pnpm dev

# 5. Start ML worker (Python)
cd apps/workers && uv run python -m src.worker
```

### Environment Setup

Copy `.env.example` to `.env` and fill in values. For local dev, defaults work with Docker Compose services.

### Useful Commands

| Command | What It Does |
|---|---|
| `make dev-api` | Start Rust API with hot reload (cargo-watch) |
| `make migrate` | Run database migrations |
| `make test` | Run all Rust tests |
| `make lint` | Run clippy + rustfmt check |
| `make build` | Build release binary |
| `cargo check` | Fast compile check (no binary output) |
| `pnpm dev` | Start Next.js dev server |
| `pnpm lint` | Lint frontend |
| `pnpm type-check` | TypeScript type check |
