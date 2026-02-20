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
- [Phase 1: Data Pipeline (COMPLETE)](#phase-1-data-pipeline--complete)
- [Phase 2: Training Engine (NEXT)](#phase-2-training-engine--next)
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
│   ├── DEVELOPMENT.md                   #   This file — development tracker
│   ├── phase0/
│   │   └── PHASE0_COMPLETE.md           #   Phase 0 completion report
│   └── phase1/
│       └── PHASE1_COMPLETE.md           #   Phase 1 completion report
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
│           ├── app_state.rs             #   AppState: DB pool + Redis + S3 + Temporal + Config
│           ├── error.rs                 #   AppError → JSON error envelope {"error":{...}}
│           ├── auth.rs                  #   Clerk JWT verification + dev token support
│           ├── middleware.rs            #   CORS, request ID (X-Request-Id), tracing
│           ├── temporal.rs              #   HTTP-based Temporal client (start workflows, get status)
│           ├── routes/
│           │   ├── health.rs            #     GET /health (liveness), GET /ready (readiness)
│           │   ├── projects.rs          #     CRUD: POST/GET/PUT/DELETE /api/v1/projects
│           │   ├── documents.rs         #     Multipart upload: POST /api/v1/projects/:id/documents
│           │   ├── pipeline.rs          #     POST parse, POST refine, GET status
│           │   └── datasets.rs          #     GET datasets (list, get, preview, parsed content)
│           ├── services/
│           │   ├── project_service.rs   #     Business logic (validation, orchestration)
│           │   ├── document_service.rs  #     Upload → S3 → DB, uses ObjectStorage trait
│           │   ├── pipeline_service.rs  #     Parse/refine triggers, pipeline status aggregation
│           │   └── dataset_service.rs   #     Dataset CRUD, S3 preview, presigned URLs
│           ├── repositories/
│           │   ├── project_repo.rs      #     SQL queries (ALL require tenant_id — enforced)
│           │   ├── document_repo.rs     #     SQL queries (ALL require tenant_id — enforced)
│           │   └── dataset_repo.rs      #     Dataset SQL queries (tenant-scoped)
│           └── dto/
│               ├── common.rs            #     PaginationParams, PaginatedResponse<T>
│               ├── project.rs           #     CreateProject, UpdateProject, ProjectResponse
│               ├── document.rs          #     UploadResponse, DocumentResponse
│               ├── dataset.rs           #     DatasetResponse
│               └── pipeline.rs          #     TriggerParse/RefineResponse, PipelineStatus
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
│   │       │           └── [id]/
│   │       │               ├── page.tsx #       Project detail (upload, pipeline, status, actions)
│   │       │               └── dataset/ #       Dataset review (ChatML pair preview, stats)
│   │       ├── hooks/
│   │       │   ├── use-projects.ts      #   React Query hooks (list, get, create, delete)
│   │       │   ├── use-documents.ts     #   Document list + upload hooks with polling
│   │       │   ├── use-pipeline.ts      #   Pipeline status + triggers with smart polling
│   │       │   └── use-datasets.ts      #   Dataset list, detail, preview hooks
│   │       └── lib/
│   │           ├── api-client.ts        #   Typed fetch wrapper with auth (projects, docs, pipeline, datasets)
│   │           └── utils.ts             #   cn() helper (clsx + tailwind-merge)
│   │
│   └── workers/                         # Python Temporal ML workers
│       ├── Dockerfile                   #   2-stage (uv build → python:3.11-slim runtime)
│       ├── pyproject.toml               #   temporalio, pydantic-settings, parsing + LLM deps
│       └── src/
│           ├── config.py                #   Worker settings (Temporal, DB, S3, Redis, LLM API)
│           ├── worker.py                #   Temporal worker entrypoint (init clients, register all)
│           ├── clients.py               #   Shared S3/DB/Redis clients (module-level singletons)
│           ├── s3_paths.py              #   Tenant-scoped S3 path builders (mirrors Rust)
│           ├── activities/
│           │   ├── stubs.py             #   Stub activities (train, evaluate, deploy — Phase 2+)
│           │   ├── parse_document.py    #   Document parsing (PDF, DOCX, HTML, MD, TXT, CSV)
│           │   ├── chunk_text.py        #   Recursive text chunking with overlap
│           │   ├── generate_pairs.py    #   LLM-powered synthetic pair generation
│           │   └── build_dataset.py     #   Quality filter, ChatML format, train/val split
│           └── workflows/
│               ├── ingest.py            #   Upload → Parse documents (implemented)
│               ├── refine.py            #   Chunk → Generate → Build dataset (implemented)
│               ├── train.py             #   Dataset → Fine-tuned LoRA adapter (stub)
│               ├── evaluate.py          #   Model → Scores + evaluation report (stub)
│               └── full_pipeline.py     #   Chains all stages end-to-end (fixed)
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
| **Phase 1: Data Pipeline** | **COMPLETE** | Document parsing, synthetic data generation, dataset building |
| **Phase 2: Training Engine** | **NEXT** | Unsloth/TRL fine-tuning, GPU orchestration |
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

| Method | Path | Description | Phase |
|---|---|---|---|
| `GET` | `/health` | Liveness check | 0 |
| `GET` | `/ready` | Readiness check (DB + Redis) | 0 |
| `POST` | `/api/v1/projects` | Create project | 0 |
| `GET` | `/api/v1/projects` | List projects (paginated) | 0 |
| `GET` | `/api/v1/projects/:id` | Get project | 0 |
| `PUT` | `/api/v1/projects/:id` | Update project | 0 |
| `DELETE` | `/api/v1/projects/:id` | Soft-delete project | 0 |
| `POST` | `/api/v1/projects/:id/documents` | Upload document (multipart → S3) | 0 |
| `GET` | `/api/v1/projects/:id/documents` | List documents | 0 |
| `GET` | `/api/v1/documents/:id` | Get document | 0 |
| `POST` | `/api/v1/projects/:id/parse` | Trigger IngestWorkflow | 1 |
| `POST` | `/api/v1/projects/:id/refine` | Trigger RefineWorkflow | 1 |
| `GET` | `/api/v1/projects/:id/status` | Aggregate pipeline status | 1 |
| `GET` | `/api/v1/projects/:id/datasets` | List datasets (paginated) | 1 |
| `GET` | `/api/v1/datasets/:id` | Get single dataset | 1 |
| `GET` | `/api/v1/datasets/:id/preview` | Preview dataset rows | 1 |
| `GET` | `/api/v1/documents/:id/parsed` | Presigned URL for parsed content | 1 |

### Temporal Workflows

| Workflow | Stages | Status |
|---|---|---|
| `IngestWorkflow` | get_document_info → parse_document (per doc) | Implemented (Phase 1) |
| `RefineWorkflow` | chunk_text → generate_pairs → build_dataset | Implemented (Phase 1) |
| `FullPipelineWorkflow` | Ingest → Refine → Train → Evaluate → Deploy | Fixed (Phase 1) |
| `TrainWorkflow` | start_training | Stub (Phase 2) |
| `EvaluateWorkflow` | run_evaluation | Stub (Phase 3) |

### Verification Checklist

- [x] `cargo check` — Rust workspace compiles (only dead_code warnings for scaffolded stubs)
- [x] All 78 source files created across Rust, TypeScript, Python
- [x] 3 Dockerfiles build independently
- [x] CI pipeline covers all 3 languages
- [x] Every DB query enforces `tenant_id`
- [x] ObjectStorage trait allows swapping S3 backend
- [x] Dev token auth works without Clerk for local development

---

## Phase 1: Data Pipeline — COMPLETE

> Upload a document, parse it, generate training data, review the dataset.

### What Was Built

| Step | Component | What It Does |
|---|---|---|
| Worker infra | `clients.py`, `s3_paths.py` | Shared S3/DB/Redis clients, tenant-scoped path builders |
| Document parsing | `activities/parse_document.py` | PDF (PyMuPDF), DOCX (python-docx), HTML, MD, TXT, CSV — with quality scoring + language detection |
| IngestWorkflow | `workflows/ingest.py` | get_document_info → parse_document per doc, partial failure tolerant |
| Temporal client | `crates/api/src/temporal.rs` | HTTP-based (no gRPC), start_ingest/start_refine/get_status |
| API endpoints | `routes/pipeline.rs`, `routes/datasets.rs` | 7 new endpoints: parse/refine triggers, pipeline status, dataset CRUD + preview |
| Text chunking | `activities/chunk_text.py` | Recursive splitting (paragraphs → sentences), configurable size + overlap |
| Pair generation | `activities/generate_pairs.py` | OpenAI-compatible LLM API, 3 task types, provider-agnostic |
| Dataset building | `activities/build_dataset.py` | Quality filtering, dedup, ChatML format, 90/10 train/val split |
| Frontend upload | `projects/[id]/page.tsx` | Drag-drop upload, doc list with status badges, pipeline status cards |
| Dataset review | `projects/[id]/dataset/page.tsx` | ChatML pair preview, stats grid, status badges |
| Status polling | `use-pipeline.ts` | React Query 3s polling when active, auto-stop when idle |

### API Endpoints (Phase 1)

| Method | Path | Description |
|---|---|---|
| `POST` | `/api/v1/projects/:id/parse` | Trigger IngestWorkflow for unparsed docs |
| `POST` | `/api/v1/projects/:id/refine` | Trigger RefineWorkflow for parsed docs |
| `GET` | `/api/v1/projects/:id/status` | Aggregate pipeline status counts |
| `GET` | `/api/v1/projects/:id/datasets` | List datasets (paginated) |
| `GET` | `/api/v1/datasets/:id` | Get single dataset |
| `GET` | `/api/v1/datasets/:id/preview` | Preview first N rows from JSONL |
| `GET` | `/api/v1/documents/:id/parsed` | Presigned URL for parsed content |

### Temporal Workflows (Implemented)

| Workflow | Stages | Status |
|---|---|---|
| `IngestWorkflow` | `get_document_info` → `parse_document` per doc | Implemented |
| `RefineWorkflow` | `chunk_text` → `generate_pairs` → `build_dataset` | Implemented |
| `FullPipelineWorkflow` | Ingest → Refine → Train → Evaluate → Deploy | Fixed (uses RefineWorkflow result) |

### Verification

- [x] `cargo clippy -- -D warnings` — zero warnings
- [x] `cargo test --workspace` — 20 tests pass
- [x] `ruff check src/ && ruff format --check src/` — Python clean
- [x] `tsc --noEmit && eslint src/` — Frontend clean
- [x] All 6 document formats parse correctly
- [x] 7 new API endpoints follow Route → Service → Repository pattern
- [x] All new DB queries enforce `tenant_id`
- [x] Smart polling stops automatically when pipeline is idle

---

## Phase 2: Training Engine — NEXT

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
