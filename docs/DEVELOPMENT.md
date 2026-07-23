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
- [Phase 2: Training Engine (COMPLETE)](#phase-2-training-engine--complete)
- [Phase 3: Evaluation & Deployment (COMPLETE)](#phase-3-evaluation--deployment--complete)
- [Architecture Hardening (COMPLETE)](#architecture-hardening--complete)
- [Phase 4: Product Polish (COMPLETE)](#phase-4-product-polish--complete)
- [Phase 5: Serving & Deployment (COMPLETE)](#phase-5-serving--deployment--complete)
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
│   ├── phase1/
│   │   └── PHASE1_COMPLETE.md           #   Phase 1 completion report
│   ├── phase3/
│   │   └── ARCHITECTURE_REVIEW.md       #   Architecture review + 19 fixes applied
│   └── phase5/
│       └── PHASE5_COMPLETE.md           #   Phase 5 completion report
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
│   │       ├── constants.rs             #   Temporal queues, Redis keys, GPU rates, upload limits
│   │       ├── s3_paths.rs              #   Tenant-scoped S3 path builders (with tests)
│   │       ├── types.rs                 #   Typed JSON structs (Hyperparams, EvaluationScores, etc.)
│   │       └── events.rs               #   Pipeline event structs for message bus
│   │
│   ├── db/                              # Database layer (SQLx + PostgreSQL)
│   │   └── src/
│   │       ├── models.rs                #   10 SQLx FromRow structs (Tenant → ModelExport)
│   │       ├── migrations/
│   │       │   ├── 001_initial_schema.sql  # Full schema: 9 tables, indexes, RLS, triggers
│   │       │   └── 002_rls_policies_and_indexes.sql  # RLS policies + composite indexes
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
│           ├── temporal.rs              #   WorkflowOrchestrator trait + TemporalClient impl
│           ├── auth_api_key.rs          #   API key auth extractor (Bearer pl_sk_...)
│           ├── routes/
│           │   ├── health.rs            #     GET /health (liveness), GET /ready (readiness)
│           │   ├── projects.rs          #     CRUD: POST/GET/PUT/DELETE /api/v1/projects
│           │   ├── documents.rs         #     Multipart upload: POST /api/v1/projects/:id/documents
│           │   ├── pipeline.rs          #     POST parse, POST refine, GET status
│           │   ├── datasets.rs          #     GET datasets (list, get, preview, parsed content)
│           │   ├── training.rs          #     Training job CRUD + cancel
│           │   ├── evaluations.rs       #     Create/list/get evaluations
│           │   ├── api_keys.rs          #     Create/list/revoke API keys
│           │   ├── deployments.rs       #     Deploy/undeploy models, status
│           │   ├── inference.rs         #     POST /v1/chat/completions (OpenAI-compatible, SSE)
│           │   ├── exports.rs           #     GGUF export: create, list, download
│           │   └── billing.rs           #     Usage summary + billing events
│           ├── services/
│           │   ├── project_service.rs   #     Business logic (validation, orchestration)
│           │   ├── document_service.rs  #     Upload → S3 → DB, uses ObjectStorage trait
│           │   ├── pipeline_service.rs  #     Parse/refine triggers, pipeline status aggregation
│           │   ├── dataset_service.rs   #     Dataset CRUD, S3 preview, presigned URLs
│           │   ├── training_job_service.rs #   Training job creation, Temporal dispatch
│           │   ├── evaluation_service.rs #    Evaluation creation, Temporal dispatch
│           │   ├── api_key_service.rs   #     Key generation, SHA-256 hashing, rate limiting
│           │   ├── deployment_service.rs #    vLLM adapter management
│           │   ├── circuit_breaker.rs   #     Async circuit breaker for vLLM
│           │   ├── billing_batcher.rs   #     Channel-based billing micro-batcher
│           │   ├── token_estimator.rs   #     Token count + cost estimation
│           │   └── export_service.rs    #     GGUF export validation + Temporal trigger
│           ├── repositories/
│           │   ├── project_repo.rs      #     SQL queries (ALL require tenant_id — enforced)
│           │   ├── document_repo.rs     #     SQL queries (ALL require tenant_id — enforced)
│           │   ├── dataset_repo.rs      #     Dataset SQL queries (tenant-scoped)
│           │   ├── training_job_repo.rs #     Training job CRUD + workflow tracking
│           │   ├── model_repo.rs        #     Model CRUD + deployment status
│           │   ├── evaluation_repo.rs   #     Evaluation CRUD + score storage
│           │   ├── api_key_repo.rs      #     Key hash lookup, revocation
│           │   ├── billing_event_repo.rs #    Append-only billing events + usage queries
│           │   └── export_repo.rs       #    Model export CRUD
│           └── dto/
│               ├── common.rs            #     PaginationParams, PaginatedResponse<T>
│               ├── project.rs           #     CreateProject, UpdateProject, ProjectResponse
│               ├── document.rs          #     UploadResponse, DocumentResponse
│               ├── dataset.rs           #     DatasetResponse
│               ├── pipeline.rs          #     TriggerParse/RefineResponse, PipelineStatus
│               ├── training_job.rs      #     CreateTrainingJob, TrainingJobResponse
│               ├── evaluation.rs        #     CreateEvaluation, EvaluationResponse
│               ├── api_key.rs           #     CreateApiKey, ApiKeyResponse
│               ├── billing.rs           #     BillingEventResponse, UsageSummary
│               └── export.rs            #     ExportRequest, ExportResponse
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
│   │       │               ├── components/ #    Extracted: StatusBadge, DocStatusBadge, etc.
│   │       │               ├── dataset/ #       Dataset review (ChatML pair preview, stats)
│   │       │               ├── training/ #      Training dashboard (create, metrics, progress)
│   │       │               └── models/
│   │       │                   └── [modelId]/
│   │       │                       ├── page.tsx       # Model detail + deploy/API keys
│   │       │                       ├── evaluation/    # Evaluation scores + charts
│   │       │                       └── playground/    # Chat playground
│   │       ├── hooks/
│   │       │   ├── use-authed-query.ts  #   Hook factory: useAuthedQuery/useAuthedMutation
│   │       │   ├── use-projects.ts      #   React Query hooks (list, get, create, delete)
│   │       │   ├── use-documents.ts     #   Document list + upload hooks with polling
│   │       │   ├── use-pipeline.ts      #   Pipeline status + triggers with smart polling
│   │       │   ├── use-datasets.ts      #   Dataset list, detail, preview hooks
│   │       │   ├── use-training.ts      #   Training job hooks (list, create, cancel)
│   │       │   ├── use-evaluations.ts   #   Evaluation hooks (list, get, create, polling)
│   │       │   ├── use-deployments.ts   #   Deploy/undeploy hooks
│   │       │   ├── use-api-keys.ts      #   API key hooks (list, create, revoke)
│   │       │   └── use-exports.ts       #   GGUF export hooks (list, create, download)
│   │       └── lib/
│   │           ├── api-client.ts        #   Typed fetch with auth, timeout, retry
│   │           ├── query-keys.ts        #   Centralized query key factories
│   │           └── utils.ts             #   cn() helper (clsx + tailwind-merge)
│   │
│   └── workers/                         # Python Temporal ML workers
│       ├── Dockerfile                   #   2-stage (uv build → python:3.11-slim runtime)
│       ├── pyproject.toml               #   temporalio, pydantic-settings, parsing + ML deps
│       └── src/
│           ├── config.py                #   Worker settings (Temporal, DB, S3, Redis, LLM API)
│           ├── worker.py                #   Temporal worker entrypoint (GPU + default queues)
│           ├── clients.py               #   Compatibility layer → delegates to InfraContainer
│           ├── infra.py                 #   Protocol-based DI (ObjectStore, Database, InfraContainer)
│           ├── s3_paths.py              #   Tenant-scoped S3 path builders (mirrors Rust)
│           ├── activities/
│           │   ├── stubs.py             #   Dataclass I/O + re-exports for Temporal registration
│           │   ├── parse_document.py    #   Document parsing (PDF, DOCX, HTML, MD, TXT, CSV)
│           │   ├── chunk_text.py        #   Recursive text chunking with overlap
│           │   ├── generate_pairs.py    #   LLM-powered synthetic pair generation
│           │   ├── build_dataset.py     #   Quality filter, ChatML format, train/val split
│           │   ├── train_model.py       #   4-mode training (quick/aligned/reasoning/iterative)
│           │   ├── run_evaluation.py    #   4-suite eval (domain/general/A-B/safety)
│           │   ├── export_gguf.py       #   GGUF merge + quantize + S3 upload
│           │   ├── training_engine.py   #   TrainingEngine Protocol + UnslothEngine impl
│           │   ├── llm_judge.py         #   LLMJudge Protocol + OpenAICompatibleJudge impl
│           │   ├── benchmark_source.py  #   BenchmarkSource Protocol + local file impl
│           │   └── benchmarks/
│           │       ├── general_benchmark.json   # 200 general capability questions
│           │       └── safety_prompts.json      # 30 safety evaluation prompts
│           └── workflows/
│               ├── ingest.py            #   Upload → Parse documents (implemented)
│               ├── refine.py            #   Chunk → Generate → Build dataset (implemented)
│               ├── train.py             #   Dataset → Fine-tuned LoRA adapter (implemented)
│               ├── evaluate.py          #   Model → 4-suite evaluation report (implemented)
│               ├── full_pipeline.py     #   Chains all stages end-to-end
│               └── export.py            #   GGUF export workflow
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

- **ObjectStorage trait** — S3 backend is swappable (AWS S3, Cloudflare R2, MinIO)
- **WorkflowOrchestrator trait** — Temporal is swappable for any orchestrator (object-safe via BoxFuture)
- **TrainingEngine Protocol** — Unsloth is swappable for any ML backend (PEFT, axolotl, etc.)
- **Multi-tenancy: repo + RLS** — every query requires `tenant_id` (primary isolation), with PostgreSQL Row-Level Security as an enforced second layer. See "Database roles & tenant isolation" below.
- **Platform admin** — infrastructure/admin endpoints (e.g. the inference-instance fleet) require a platform admin, not a tenant `Owner`. Grant via the `PLATFORM_ADMIN_USER_IDS` (JWT `sub`) / `PLATFORM_ADMIN_EMAILS` allowlists; both empty = deny-all.
- **Pluggable email & billing** — `EmailProvider` (SMTP, via `SMTP_*`) and `BillingProvider` (Stripe, via `STRIPE_*`) sit behind traits; a provider is used when configured, otherwise an honest no-op that returns errors instead of faking success. Any SMTP provider works (Resend/SES/Brevo/SendGrid — no self-hosted mail server); email deliveries fail visibly when SMTP is unset.
- **Dual auth** — Clerk JWT for platform users, API keys for model consumers (separate extractors)
- **Dev token auth** — `dev_{tenant_uuid}_{user_id}` format for local development without Clerk
- **Soft deletes** — projects use `deleted_at` instead of hard delete
- **Strategy pattern** — training modes and evaluation suites are registered classes, not if-elif chains
- **Protocol-based DI** — Python workers use `InfraContainer` with typed protocols, not globals

### Database roles & tenant isolation

Tenant isolation uses two PostgreSQL connections with different privileges:

- **Owner (`DATABASE_URL`)** — owns the schema. Runs migrations, billing-partition
  DDL, admin/infra endpoints, and the few genuinely cross-tenant operations
  (batched billing writes, the stale-deployment reaper, the global adapter-cap
  count, auth-by-hash / invitation acceptance before a tenant is known). Bypasses
  RLS by virtue of ownership.
- **`app_rls` (`DATABASE_RLS_URL`)** — a least-privilege role (`NOSUPERUSER
  NOBYPASSRLS`, not the owner) created by migration `017`. Carries all tenant
  request traffic. Tenant-scoped repository methods run each query inside a
  transaction that sets `app.tenant_id` (`begin_tenant_tx`), so RLS filters rows
  by tenant and **fails closed** (zero rows) when no tenant context is set.

`WHERE tenant_id` stays in every query as the primary filter; RLS is an enforced
second layer. At startup the API asserts the RLS connection is actually subject
to RLS (`row_security_active`) and refuses to boot otherwise. If
`DATABASE_RLS_URL` is unset, tenant traffic falls back to the owner connection
and the RLS layer is inactive (a startup warning is emitted) — set it in every
real deployment.

**Provisioning `app_rls`:**
- **Dev** — migration `017` creates the role automatically with a local-only
  default password; `DATABASE_RLS_URL` in `.env.example` matches it.
- **Production** — create the role with a STRONG password *before* running
  migrations (migration `017` skips creation if it already exists), then set
  `APP_RLS_PASSWORD` / `DATABASE_RLS_URL`:
  ```sql
  CREATE ROLE app_rls LOGIN PASSWORD '<strong>' NOSUPERUSER NOBYPASSRLS NOCREATEDB NOCREATEROLE;
  ```

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
| **Phase 0: Foundation** | **COMPLETE** | Infrastructure skeleton |
| **Phase 1: Data Pipeline** | **COMPLETE** | Document parsing, synthetic data generation, dataset building |
| **Phase 2: Training Engine** | **COMPLETE** | Unsloth/TRL fine-tuning, 4 training modes, GPU orchestration |
| **Phase 3: Eval & Deploy** | **COMPLETE** | 4-suite evaluation, vLLM deployment, API keys, inference proxy |
| **Architecture Hardening** | **COMPLETE** | 19 fixes: trait abstractions, RLS, indexes, DI, typed JSON |
| **Phase 4: Product Polish** | **COMPLETE** | Team RBAC, Stripe billing, dashboard, notifications, onboarding |
| **Phase 5: Serving & Deploy** | **COMPLETE** | Circuit breaker, SSE streaming, GGUF export, usage metering |

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

## Phase 2: Training Engine — COMPLETE

> Fine-tune LLMs with 4 training modes — no ML expertise required.

### What Was Built

| Step | Component | What It Does |
|---|---|---|
| Training activity | `activities/train_model.py` | Full Unsloth/TRL training with Strategy pattern dispatch |
| Training engine | `activities/training_engine.py` | `TrainingEngine` Protocol + `UnslothEngine` impl (swappable ML backend) |
| 4 training modes | Quick (SFT), Aligned (DPO), Reasoning (GRPO), Iterative (multi-round SFT) | Each mode is a registered `TrainingStrategy` class |
| LLM-as-judge DPO | `_create_dpo_pairs_with_judge()` | LLM scores responses for chosen/rejected pair creation |
| LLM reward (GRPO) | `_llm_reward()` | LLM-based reward function for reinforcement learning |
| Checkpoint upload | `CheckpointUploadCallback` | Saves checkpoints to S3 every N steps for resume |
| GPU monitoring | `_get_gpu_metrics()` via pynvml | Streams GPU utilization, VRAM, temperature to Redis |
| Metrics streaming | Redis SSE | Real-time loss, learning rate, GPU metrics to frontend |
| Hyperparameter mgmt | `merge_hyperparams()` | Smart defaults + user overrides |
| Cost estimation | `estimate_cost()` | Heuristic based on model size, dataset, GPU class |
| TrainWorkflow | `workflows/train.py` | GPU queue, 6hr timeout, heartbeat monitoring |
| Rust API layer | Routes + service + repo + DTO | Training job CRUD, cancel, Temporal dispatch |
| Frontend | Training dashboard page | Create job, live metrics via SSE, progress tracking |

### Training Modes

| Mode | Method | Use Case |
|---|---|---|
| **Quick** | SFT (Supervised Fine-Tuning) | Fast domain adaptation, instruction following |
| **Aligned** | DPO (Direct Preference Optimization) | Preference alignment, safety tuning |
| **Reasoning** | GRPO (Group Relative Policy Optimization) | Chain-of-thought, math reasoning |
| **Iterative** | Multi-round SFT with validation | Highest quality, uses hold-out eval between iterations |

### API Endpoints (Phase 2)

| Method | Path | Description |
|---|---|---|
| `POST` | `/api/v1/projects/:id/training-jobs` | Create training job (auto-triggers workflow) |
| `GET` | `/api/v1/projects/:id/training-jobs` | List training jobs (paginated) |
| `GET` | `/api/v1/training-jobs/:id` | Get training job |
| `POST` | `/api/v1/training-jobs/:id/cancel` | Cancel training job |
| `GET` | `/api/v1/training-jobs/:id/metrics/stream` | SSE metrics stream |

---

## Phase 3: Evaluation & Deployment — COMPLETE

> Evaluate model quality with 4 suites, deploy for inference, serve via API keys.

### What Was Built — Evaluation

| Step | Component | What It Does |
|---|---|---|
| Evaluation activity | `activities/run_evaluation.py` | 4-suite evaluation engine with suite registry |
| LLM Judge | `activities/llm_judge.py` | `LLMJudge` Protocol + `OpenAICompatibleJudge` (unified) |
| Benchmark source | `activities/benchmark_source.py` | `BenchmarkSource` Protocol for loading test datasets |
| Domain suite | Validates on held-out data | LLM judge scores accuracy, completeness, faithfulness |
| General suite | Tests broad capabilities | 200 questions across reasoning, math, coding, knowledge |
| A/B comparison | Base vs fine-tuned blind test | Win rate + 95% Wilson confidence interval |
| Safety suite | Checks for regression | 30 prompts: harmful requests, jailbreaks, bias |
| EvaluateWorkflow | `workflows/evaluate.py` | GPU queue, 1hr timeout, judge config passthrough |
| Rust API layer | Routes + service + repo + DTO | Evaluation CRUD, Temporal dispatch |
| Frontend | Evaluation page | Score cards, charts, recommendations, run button |

### What Was Built — Deployment & Inference

| Step | Component | What It Does |
|---|---|---|
| Deployment service | `services/deployment_service.rs` | vLLM adapter lifecycle (load/unload via REST API) |
| Inference proxy | `routes/inference.rs` | OpenAI-compatible `/v1/chat/completions` |
| API key system | `services/api_key_service.rs` | `pl_sk_` prefix, SHA-256 hash, shown once |
| API key auth | `auth_api_key.rs` | Axum extractor for `Bearer pl_sk_...` |
| Rate limiting | Redis sliding window | Per-minute limits via `rl:{key_id}:{minute}` |
| Billing metering | `repositories/billing_event_repo.rs` | Append-only token + cost tracking |
| Frontend | Model page, playground | Deploy toggle, API key management, chat playground |

### API Endpoints (Phase 3)

| Method | Path | Auth | Description |
|---|---|---|---|
| `POST` | `/api/v1/models/:id/evaluations` | Clerk JWT | Create evaluation |
| `GET` | `/api/v1/models/:id/evaluations` | Clerk JWT | List evaluations |
| `GET` | `/api/v1/evaluations/:id` | Clerk JWT | Get evaluation |
| `POST` | `/api/v1/models/:id/deploy` | Clerk JWT | Deploy model to vLLM |
| `POST` | `/api/v1/models/:id/undeploy` | Clerk JWT | Undeploy model |
| `GET` | `/api/v1/models/:id/deployment` | Clerk JWT | Get deployment status |
| `POST` | `/api/v1/models/:id/api-keys` | Clerk JWT | Create API key |
| `GET` | `/api/v1/models/:id/api-keys` | Clerk JWT | List API keys |
| `POST` | `/api/v1/api-keys/:id/revoke` | Clerk JWT | Revoke API key |
| `POST` | `/v1/chat/completions` | API Key | Inference (OpenAI-compatible) |
| `GET` | `/api/v1/billing/usage` | Clerk JWT | Usage summary |
| `GET` | `/api/v1/billing/events` | Clerk JWT | List billing events |
| `POST` | `/api/v1/models/:id/exports` | Clerk JWT | Start GGUF export |
| `GET` | `/api/v1/models/:id/exports` | Clerk JWT | List exports for model |
| `GET` | `/api/v1/exports/:id/download` | Clerk JWT | Presigned download URL |
| `GET` | `/api/v1/dashboard/inference-usage` | Clerk JWT | 30-day inference usage |

### Temporal Workflows (All Implemented)

| Workflow | Stages | Queue |
|---|---|---|
| `IngestWorkflow` | get_document_info → parse_document | default |
| `RefineWorkflow` | chunk_text → generate_pairs → build_dataset | default |
| `TrainWorkflow` | start_training (6hr timeout) | gpu |
| `EvaluateWorkflow` | run_evaluation (1hr timeout) | gpu |
| `FullPipelineWorkflow` | Ingest → Refine → Train → Evaluate → Deploy | default → gpu |
| `TrainIterativeWorkflow` | Multi-round SFT with holdout eval + early stopping | gpu |
| `TrainReasoningWorkflow` | SFT → GRPO reasoning optimization | gpu |
| `TrainAlignedWorkflow` | SFT → DPO preference alignment | gpu |
| `ExportWorkflow` | GGUF merge → convert → quantize → S3 upload | default |

---

## Architecture Hardening — COMPLETE

> 19 fixes across all layers — trait abstractions, security, performance, DX.

### Fixes Applied

| ID | Priority | Fix | Layer |
|---|---|---|---|
| P0-1 | Critical | `WorkflowOrchestrator` trait — swap Temporal for any orchestrator | Rust API |
| P0-2 | Critical | `TrainingEngine` Protocol — swap Unsloth for any ML backend | Python |
| P0-3 | Critical | Training Strategy pattern — registered strategies vs if-elif | Python |
| P0-4 | Critical | Evaluation Suite registry — plugin architecture for eval suites | Python |
| P1-1 | High | Typed JSON structs (`Hyperparams`, `EvaluationScores`, etc.) | Rust shared |
| P1-2 | High | RLS policies on all 8 tenant-scoped tables | PostgreSQL |
| P1-3 | High | 11 composite indexes for common query patterns | PostgreSQL |
| P1-4 | High | Protocol-based DI (`InfraContainer`) for Python workers | Python |
| P1-5 | High | AppState uses trait objects (`dyn WorkflowOrchestrator`) | Rust API |
| P2-2 | Medium | Extracted 5 inline components from monolithic page.tsx | Frontend |
| P2-3 | Medium | `useAuthedQuery`/`useAuthedMutation` hook factories | Frontend |
| P2-5 | Medium | `BenchmarkSource` Protocol for evaluation data loading | Python |
| P2-7 | Medium | Unified `LLMJudge` Protocol consolidating 3 scattered impls | Python |
| P2-8 | Medium | `fetchWithRetry` with AbortController timeout + exponential backoff | Frontend |
| P2-9 | Medium | Error logging on all fire-and-forget `tokio::spawn` blocks | Rust API |
| P2-10 | Medium | Typed structs for all JSON blob fields with serde support | Rust shared |
| P3-1 | Low | Centralized query key factories | Frontend |
| P3-2 | Low | Config validation for environment variables | Python |
| P3-3 | Low | Adapter naming conventions standardized | Python |

### Key Design Decisions

- **WorkflowOrchestrator trait** uses `BoxFuture` pattern for object safety (not RPITIT)
- **Strategy/Suite registries** use decorator-based registration (`@register_strategy`)
- **InfraContainer** replaces module-level global singletons with typed Protocol-based DI
- **RLS policies** use `current_setting('app.tenant_id', true)::uuid` — defense-in-depth with repo-layer enforcement
- **Typed JSON structs** use `#[serde(flatten)] extra: HashMap<String, Value>` for forward compatibility

---

## Phase 4: Product Polish — COMPLETE

> Production-ready billing, team management, onboarding.

### Work Items

| Task | Description | Status |
|---|---|---|
| Stripe billing integration | Usage-based pricing tied to billing_events, webhook handlers with HMAC verification | **COMPLETE** |
| Team management | Invite members, roles (admin/member/viewer), RBAC middleware | **COMPLETE** |
| Onboarding flow | Guided first-project experience with step completion tracking | **COMPLETE** |
| Usage dashboard | Dashboard stats overview with Redis caching (30s/60s TTL) | **COMPLETE** |
| Notifications | Webhook delivery with SSRF protection, notification preferences | **COMPLETE** |
| Audit log | Append-only audit_logger service for tracking actions | **COMPLETE** |

See `docs/phase4/PHASE4A_COMPLETE.md` and `docs/phase4/PHASE4BC_COMPLETE.md` for details.

---

## Phase 5: Serving & Deployment — COMPLETE

> Production-ready inference. Circuit breaker, SSE streaming, GGUF export, usage metering.

### Work Items

| Task | Description | Status |
|---|---|---|
| Circuit breaker | Async state machine protecting vLLM calls (503 on open) | **COMPLETE** |
| Billing micro-batching | Channel + background worker, bulk inserts (10K req/min → 12 inserts/min) | **COMPLETE** |
| SSE streaming inference | Forward vLLM byte stream, usage extraction from final chunk | **COMPLETE** |
| Frontend streaming | ReadableStream playground with incremental token display | **COMPLETE** |
| Real deploy activity | Python activity calls Rust API with auth token | **COMPLETE** |
| GGUF export pipeline | Temporal workflow: merge LoRA → GGUF → quantize → S3 | **COMPLETE** |
| Usage dashboard page | Inference usage by day, bar charts, daily breakdown table | **COMPLETE** |
| Token estimator | Centralized token/cost estimation for billing | **COMPLETE** |
| Dashboard Redis cache | 30s/60s TTL to prevent DB pool exhaustion | **COMPLETE** |

See `docs/phase5/PHASE5_COMPLETE.md` for details.

---

## Phase 6: Polish & Scale — Status

> Production-grade, scalable, polished.

Most Phase 6 items were completed as part of earlier phases. Remaining items are optional scale/ops tasks.

| Task | Description | Status | Notes |
|---|---|---|---|
| Usage tracking & metering | Token counting, request tracking, cost attribution | **COMPLETE** | Done in Phase 5 (billing_batcher + usage page) |
| Cost estimation | Per-operation cost calculation | **COMPLETE** | Done in Phase 5 (token_estimator.rs) |
| GRPO training mode | Reasoning-optimized SFT → GRPO | **COMPLETE** | Done in Phase 2 (train_reasoning.py) |
| Iterative training mode | Multi-round SFT with holdout eval + early stopping | **COMPLETE** | Done in Phase 2 (train_iterative.py) |
| Webhook system | Stripe webhooks + custom notification webhooks | **COMPLETE** | Done in Phase 4 (stripe_webhooks.rs, notification_service.rs) |
| Multi-model base support | Support Llama, Qwen, Mistral, DeepSeek | **PARTIAL** | Architecture supports any HuggingFace model; only Llama-3.1-8B validated |
| Advanced evaluation | Custom benchmarks, pluggable suites | **COMPLETE** | Done in Phase 3 (BenchmarkSource protocol, 197-item benchmark, LLMJudge) |
| Load testing | 50+ concurrent users, performance gates | **NOT DONE** | No automated test harness; gates defined in ARCHITECTURE.md only |
| RunPod integration | Serverless GPU for cost optimization | **NOT DONE** | Documented as future; using Modal currently |
| API documentation | OpenAPI spec, endpoint reference | **COMPLETE** | utoipa v5 + Swagger UI at `/docs`. 53 endpoints, 80+ schemas, 17 tags. All handlers annotated with `#[utoipa::path]`. |

### What's Left (Optional / Post-MVP)

These are **nice-to-haves** for production scale, not blockers for the platform to function:

1. **Load testing harness** — k6 or Locust scripts for benchmarking inference at scale
2. **RunPod integration** — alternative GPU provider for cost optimization
3. **Multi-model UI** — base model selector dropdown in training form (engine already supports any model)
4. **ClickHouse migration** — move billing_events to OLAP store when Postgres query latency exceeds SLA

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
