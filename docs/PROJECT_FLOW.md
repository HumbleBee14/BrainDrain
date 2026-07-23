# BrainDrain — End-to-End Project Flow

> The single, definitive guide to understanding what BrainDrain is, how it works, and how every component connects together. Read this first.

**Last Updated:** 2026-02-22

---

## Table of Contents

1. [What is BrainDrain?](#1-what-is-braindrain)
2. [The User Journey (Non-Technical)](#2-the-user-journey)
3. [System Architecture Overview](#3-system-architecture-overview)
4. [Component Deep Dive](#4-component-deep-dive)
5. [End-to-End Data Flow](#5-end-to-end-data-flow)
6. [The Six Pipeline Stages](#6-the-six-pipeline-stages)
7. [Infrastructure & Runtime Services](#7-infrastructure--runtime-services)
8. [Authentication & Multi-Tenancy](#8-authentication--multi-tenancy)
9. [Billing & Usage Metering](#9-billing--usage-metering)
10. [Resilience & Scaling Patterns](#10-resilience--scaling-patterns)
11. [Flow Diagrams](#11-flow-diagrams)
12. [Feature Completeness Matrix](#12-feature-completeness-matrix)

---

## 1. What is BrainDrain?

BrainDrain is an **end-to-end, multi-tenant LLM fine-tuning and serving platform**. It takes a user from a raw PDF document all the way through to a deployed, production-ready fine-tuned model accessible via an OpenAI-compatible API.

**In one sentence:** Upload your documents → BrainDrain parses, generates training data, fine-tunes a model, evaluates it, deploys it, and gives you an API key to use it.

### Why Does This Exist?

The ML ecosystem is highly fragmented. ML engineers currently spend a majority of their time manually gluing together:
- Document parsers (PyMuPDF, Docling)
- Data synthesizers (raw LLM API calls, custom pipelines)
- Training frameworks (Unsloth, TRL, PEFT)
- Workflow orchestrators (Temporal, Airflow)
- Inference engines (vLLM, TGI, SGLang)
- Billing and metering systems

BrainDrain abstracts all of this into a single cohesive product for fine-tuning open-weight models (Llama 3, Qwen, Mistral, etc.).

### Serving Design

Deployed models are served through a pluggable backend abstraction over vLLM,
TGI, or SGLang, with a multi-instance control plane that can bind a model to
whichever registered instance has capacity. vLLM's multi-LoRA support (hot-
loading adapters from S3 into a single running server) is the primary path;
exact per-GPU adapter-capacity numbers depend on adapter rank, base model
size, and hardware, and have not been benchmarked on this platform — treat
any specific adapter-count figure as a vLLM capability, not a measured
platform guarantee.

---

## 2. The User Journey

Here is what a user experiences, step by step:

```
1. Sign Up          → Clerk authentication, tenant provisioned
2. Create Project   → Name, description, task type (Q&A / Instruction / Reasoning)
3. Upload Documents → Drag-and-drop PDFs, DOCX, TXT, HTML, CSV, Markdown
4. Parse Documents  → Click "Parse" → BrainDrain extracts text, tables, structure
5. Refine Data      → Click "Refine" → BrainDrain generates synthetic Q&A pairs
6. Review Dataset   → Preview ChatML training pairs, approve quality
7. Train Model      → Choose training mode → LoRA fine-tuning runs on GPU
8. Evaluate Model   → 4-suite evaluation: Domain, General, A/B, Safety
9. Deploy Model     → One-click deploy to inference backend (vLLM/TGI/SGLang)
10. Get API Key     → Generate pl_sk_... key for your deployed model
11. Use Model       → POST /v1/chat/completions (OpenAI-compatible)
12. Export Model     → Download as GGUF for local use with Ollama/llama.cpp
```

---

## 3. System Architecture Overview

It is a **monorepo with three independently deployable services**:

```
┌─────────────────────────────────────────────────────────────────────┐
│                         BrainDrain Monorepo                         │
│                                                                     │
│   ┌───────────────────┐  ┌──────────────────┐  ┌──────────────────┐ │
│   │   Rust API        │  │   Next.js Web    │  │   Python Workers │ │
│   │   (crates/api)    │  │   (apps/web)     │  │   (apps/workers) │ │
│   │                   │  │                  │  │                  │ │
│   │ • Axum HTTP       │  │ • React 19       │  │ • Temporal SDK   │ │
│   │ • SQLx + Postgres │  │ • Clerk Auth     │  │ • PyMuPDF        │ │
│   │ • S3 Client       │  │ • React Query    │  │ • Unsloth / TRL  │ │
│   │ • Redis           │  │ • TailwindCSS    │  │ • vLLM Client    │ │
│   │ • Temporal Client │  │ • SSE Streaming  │  │ • httpx (LLM)    │ │
│   │                   │  │                  │  │                  │ │
│   │ ~20MB Docker      │  │ ~50MB Docker     │  │ ~500MB+ Docker   │ │
│   └────────┬──────────┘  └────────┬─────────┘  └───────┬──────────┘ │
│            │                      │                    │            │
│            └──────────┬───────────┘────────────────────┘            │
│                       │                                             │
│         Communicate ONLY via HTTP API + Temporal Task Queues        │
└─────────────────────────────────────────────────────────────────────┘
                        │
    ┌───────────────────┼───────────────────────────┐
    │           INFRASTRUCTURE LAYER                │
    │                                               │
    │  PostgreSQL 16  │  Redis 7  │  MinIO (S3)     │
    │  Temporal.io    │  vLLM     │  (Optional OTEL)│
    └───────────────────────────────────────────────┘
```

### The Three Services

| Service | Language | Role | Scales... |
|---------|----------|------|-----------|
| **Rust API** (`crates/api`) | Rust (Axum) | Control plane: auth, CRUD, billing, proxying | Horizontally (stateless) |
| **Next.js Web** (`apps/web`) | TypeScript | User interface: dashboard, project management, playground | Horizontally (stateless) |
| **Python Workers** (`apps/workers`) | Python | Execution plane: parsing, training, evaluation, export | Vertically (GPU machines) |

**Key Design Rule:** These three services share **zero runtime dependencies**. They communicate exclusively via the Rust API's HTTP endpoints and Temporal task queues.

---

## 4. Component Deep Dive

### 4.1 Rust API — The Control Plane

The API follows a strict **3-layer architecture** with trait-based dependency injection:

```
HTTP Request
    │
    ▼
┌─────────────────────────────────────────┐
│  Route Handler (thin)                   │
│  • Extract auth (Clerk JWT or API Key)  │
│  • Validate/deserialize DTO             │
│  • Call service layer                   │
│  • Serialize response DTO               │
└──────────────┬──────────────────────────┘
               │
               ▼
┌─────────────────────────────────────────┐
│  Service Layer (business logic)         │
│  • Validation rules                     │
│  • Orchestrate repos, S3, Redis         │
│  • Trigger Temporal workflows           │
│  • RBAC enforcement (require_role())    │
└──────────────┬──────────────────────────┘
               │
               ▼
┌─────────────────────────────────────────┐
│  Repository Layer (data access)         │
│  • Pure SQL queries via SQLx            │
│  • EVERY query requires tenant_id       │
│  • Trait-based (dyn ProjectRepository)  │
└──────────────┬──────────────────────────┘
               │
               ▼
         PostgreSQL (RLS enabled)
```

**AppState** (`app_state.rs`) holds all initialized connections and trait objects:
- `PgPool` (database)
- `ConnectionManager` (Redis)
- `S3Storage` (MinIO/R2/AWS)
- `dyn WorkflowOrchestrator` (Temporal — optional, degrades gracefully)
- `reqwest::Client` (HTTP client for vLLM proxying, 10s timeout)
- 14 repository trait objects (`dyn ProjectRepository`, etc.)
- `CircuitBreaker` (vLLM protection)
- `BillingBatcher` (micro-batching channel, 10K capacity, 5s flush)
- `dyn BillingProvider` (Stripe or NoOp)
- `AuthProviderChain` (Clerk JWKS verification)

**Per-Tenant Settings:** Tenant configuration (LLM provider, API keys, model preferences) is stored in the `tenants.settings` JSONB column. The Settings API (`GET/PUT/DELETE /api/v1/settings/llm`) lets admins configure their own LLM provider — workers resolve this config from the database at activity execution time, falling back to platform env var defaults. API keys are never returned in full (masked in responses) and never appear in Temporal workflow history.

**API Documentation:** OpenAPI spec is auto-generated via `utoipa` proc macros. In non-production environments, Swagger UI is served at `/docs` for interactive API exploration. All 53+ endpoints are documented with request/response schemas, authentication requirements, and parameter descriptions.

### 4.2 Next.js Web — The User Interface

The frontend is a standard Next.js 15 app with:
- **Clerk** for authentication (sign-in, sign-up, session management)
- **React Query** for server state (with smart polling for active pipelines)
- **TailwindCSS** for styling
- **Custom hooks** (`use-projects.ts`, `use-training.ts`, etc.) that wrap the API client

Key pages:
| Page | Path | Purpose |
|------|------|---------|
| Dashboard | `/dashboard` | Stats overview (projects, models, docs) |
| Project List | `/projects` | All user projects |
| Project Detail | `/projects/[id]` | Upload docs, trigger pipeline, view status |
| Dataset Review | `/projects/[id]/dataset` | Preview ChatML pairs, stats |
| Training | `/projects/[id]/training` | Create training job, live metrics |
| Model Detail | `/projects/[id]/models/[modelId]` | Deploy, API keys, export |
| Playground | `.../models/[modelId]/playground` | Chat with your model (SSE streaming) |
| Evaluation | `.../models/[modelId]/evaluation` | Score cards, charts |
| Settings/Usage | `/settings/usage` | Token usage charts, daily breakdown |
| Settings/LLM | `/settings/llm` | Configure LLM provider, API key, model |

### 4.3 Python Workers — The Execution Plane

Workers are Temporal activities that do the actual ML computation:

```
Temporal Worker Process (src/worker.py)
    │
    │  Worker modes (APP_WORKER_MODE env var):
    │    "all"  → Dev mode — listens on "ml-pipeline" queue, registers ALL activities
    │    "main" → CPU mode — listens on "ml-pipeline-main" queue, CPU activities only
    │    "gpu"  → GPU mode — listens on "ml-pipeline-gpu" queue, GPU activities only
    │
    ├── CPU Activities (ml-pipeline-main):
    │   ├── get_document_info     → Fetch document metadata from API
    │   ├── parse_document        → PDF/DOCX/HTML/Markdown/CSV/TXT parsing
    │   ├── chunk_text            → Recursive text chunking
    │   ├── generate_synthetic_pairs → LLM-powered Q&A synthesis
    │   ├── build_dataset         → Quality filter + ChatML format
    │   ├── deploy_model          → HTTP call to Rust API
    │   └── export_gguf           → LoRA merge + GGUF convert + quantize
    │
    └── GPU Activities (ml-pipeline-gpu):
        ├── start_training    → Unsloth/TRL fine-tuning
        └── run_evaluation    → 4-suite model evaluation (LLM-Judge + benchmarks)

Workflows (9 total):
    ├── IngestWorkflow        → Document parsing pipeline
    ├── RefineWorkflow        → Synthetic data generation (chunk → pairs → dataset)
    ├── TrainWorkflow         → Basic SFT training
    ├── TrainAlignedWorkflow  → SFT → DPO alignment training
    ├── TrainIterativeWorkflow→ Multi-round iterative fine-tuning
    ├── TrainReasoningWorkflow→ SFT → GRPO reasoning training
    ├── EvaluateWorkflow      → 4-suite model evaluation
    ├── ExportWorkflow        → GGUF export + quantization
    └── FullPipelineWorkflow  → End-to-end orchestration
```

**Key Pattern:** Workers use `Protocol`-based dependency injection (`TrainingEngine Protocol`, `LLMJudge Protocol`, `BenchmarkSource Protocol`). The `UnslothEngine` is the default implementation, but any ML backend can be swapped in by implementing the same Protocol.

**Per-Tenant LLM Config:** Every activity that calls an LLM (synthetic pair generation, DPO/GRPO judge, evaluation judge) resolves the LLM provider config per-tenant from the database at execution time via `get_tenant_llm_config()`. If a tenant has configured their own provider (e.g., Groq, Anthropic, local Ollama), that config is used; otherwise, the worker falls back to platform env var defaults (`APP_LLM_API_KEY`, etc.).

---

## 5. End-to-End Data Flow

This is the complete journey of data through BrainDrain:

```
                    ┌──────────────────────────────────────────────────────────────────┐
                    │                    COMPLETE DATA FLOW                            │
                    │                                                                  │
   User uploads     │  ┌─────────┐    ┌─────────┐    ┌──────────┐    ┌────────────┐    │
   PDF/DOCX  ──────►│  │ Parse   │───►│ Chunk   │───►│Synthesize│───►│   Build    │    │
                    │  │ Document│    │ Text    │    │ Q&A Pairs│    │  Dataset   │    │
                    │  └────┬────┘    └────┬────┘    └─────┬────┘    └─────┬──────┘    │
                    │       │              │               │               │           │
                    │       ▼              ▼               ▼               ▼           │
                    │    S3: raw/       S3: parsed/     S3: pairs/      S3: datasets/  │
                    │    {tenant}/      {tenant}/       {tenant}/       {tenant}/      │
                    │    {project}/     {project}/      {project}/      {project}/     │
                    │                                                                  │
                    │  ┌──────────┐    ┌──────────┐    ┌──────────┐    ┌────────────┐  │
                    │  │  Train   │───►│ Evaluate │───►│  Deploy  │───►│  Inference │  │
                    │  │  LoRA    │    │  Model   │    │  to vLLM │    │  via API   │  │
                    │  └────┬─────┘    └─────┬────┘    └─────┬────┘    └─────┬──────┘  │
                    │       │                │               │               │         │
                    │       ▼                ▼               ▼               ▼         │
                    │    S3: adapters/   DB: eval_      vLLM: adapter    Billing:      │
                    │    {tenant}/       scores         loaded in VRAM   tokens metered│
                    └──────────────────────────────────────────────────────────────────┘
```

### Where Data Lives

| Data Type | Storage | Why |
|-----------|---------|-----|
| Uploaded files (PDF, DOCX) | S3 (MinIO) | Cheap, scalable blob storage |
| Parsed text content | S3 (MinIO) | Large text blobs, not queryable |
| Synthetic Q&A pairs | S3 (MinIO) | JSONL files, potentially GBs |
| Training datasets | S3 (MinIO) | JSONL with train/val split |
| LoRA adapters | S3 (MinIO) | Model weights (~25-30MB each) |
| GGUF exports | S3 (MinIO) | Quantized models (1-8GB each) |
| Project/document metadata | PostgreSQL | Relational, queryable, RLS-protected |
| Training job status & metrics | PostgreSQL | Transactional state |
| Billing events | PostgreSQL | Partitioned by month |
| API key hashes | PostgreSQL | SHA-256 hashed, never stored raw |
| Rate limit counters | Redis | Fast expiring counters |
| Dashboard cache | Redis | 30s/60s TTL JSON blobs |
| Workflow state | Temporal | Durable execution history |

---

## 6. The Six Pipeline Stages

### Stage 1: Document Parsing (IngestWorkflow)

**Trigger:** User clicks "Parse" → API calls `POST /api/v1/projects/:id/parse` → Temporal `IngestWorkflow` starts.

```
For each uploaded document:
  1. Download raw file from S3
  2. Detect file type (PDF, DOCX, HTML, MD, TXT, CSV)
  3. Parse with appropriate engine:
     • PDF → PyMuPDF (text + structure extraction)
     • DOCX → python-docx
     • HTML → BeautifulSoup
     • MD/TXT → Direct text
     • CSV → Pandas
  4. Generate quality score (0-100)
  5. Upload parsed content to S3 as JSON
  6. Update document status in PostgreSQL: "parsed"
```

**Files:** `apps/workers/src/activities/parse_document.py`, `workflows/ingest.py`

### Stage 2: Text Chunking (RefineWorkflow — Step 1)

```
1. Download parsed JSON from S3
2. Split into chunks using recursive strategy:
   • Split by paragraphs first
   • If chunk too large, split by sentences
   • Configurable chunk_size (default: 1500 tokens) and overlap (200 tokens)
3. Each chunk retains source metadata (document ID, page numbers)
4. Upload chunks to S3
```

**File:** `apps/workers/src/activities/chunk_text.py`

### Stage 3: Synthetic Data Generation (RefineWorkflow — Step 2)

```
1. Resolve LLM provider config for this tenant (DB lookup → env var fallback)
2. For each chunk, call the tenant's LLM via OpenAI-compatible API
3. Generate Q&A pairs grounded in the chunk content
4. Three task types supported:
   • Q&A: Factual questions about the document
   • Instruction: "Write/summarize/explain" style tasks
   • Reasoning: Multi-step analytical questions
5. Each pair includes source span for grounding verification
6. Upload raw pairs to S3
```

**LLM Config:** The tenant's LLM provider (configured via `PUT /api/v1/settings/llm`) is resolved from the database at execution time. If no custom config exists, falls back to platform defaults (`APP_LLM_API_KEY` env var). Works with any OpenAI-compatible provider (OpenAI, Groq, Anthropic, Together AI, local Ollama, etc.).

**File:** `apps/workers/src/activities/generate_pairs.py` (activity name: `generate_synthetic_pairs`)

### Stage 4: Dataset Building (RefineWorkflow — Step 3)

```
1. Load all raw pairs
2. Quality filtering (pluggable backend): default "heuristic" backend applies
   length-based rules — drops pairs with an empty/too-short instruction or a
   response that is too short or too long. This is not LLM-scored.
3. Deduplication (pluggable backend): default "hash" backend removes exact
   duplicates via an MD5 hash of instruction+response. An optional "near"
   backend (token-Jaccard near-duplicate removal) can be enabled via
   APP_DEDUP_BACKEND=near — it is not the default and is lexical, not embedding-based.
4. Format as ChatML JSONL:
   {"messages": [
     {"role": "system", "content": "..."},
     {"role": "user", "content": "..."},
     {"role": "assistant", "content": "..."}
   ]}
5. 90/10 train/validation split
6. Upload dataset to S3
7. Create dataset record in PostgreSQL
```

**File:** `apps/workers/src/activities/build_dataset.py`

### Stage 5: Model Training (TrainWorkflow)

**Trigger:** User creates training job → API calls `POST /api/v1/projects/:id/training-jobs` → Temporal `TrainWorkflow` starts on GPU queue.

```
1. Download dataset from S3
2. Load base model (e.g., Llama-3.1-8B) via Unsloth
3. Configure LoRA (rank, alpha, target modules)
4. Execute training based on selected mode:
   • Quick (SFT only): Fastest, single-pass supervised fine-tuning
   • Aligned (SFT → DPO): Preference alignment using LLM-generated pairs
   • Reasoning (SFT → GRPO): Reinforcement learning for chain-of-thought
   • Iterative: Multi-round SFT with validation between iterations
5. Save checkpoints to S3 every N steps
6. Stream metrics (loss, learning rate, GPU utilization) to Redis → SSE → Frontend
7. Save final LoRA adapter to S3
8. Create model record in PostgreSQL
```

**Files:** `apps/workers/src/activities/train_model.py`, `training_engine.py`

### Stage 6: Evaluation, Deployment & Serving

**Evaluation** (`EvaluateWorkflow`):
```
Four evaluation suites run in parallel:
1. Domain Suite:  LLM-Judge scores on held-out test data (accuracy, faithfulness)
2. General Suite: 196 broad-capability questions (detect catastrophic forgetting)
3. A/B Suite:     Blind comparison — base model vs fine-tuned (win rate)
4. Safety Suite:  65 harmful/bias prompts (detect safety regression)
```

**Deployment** (`POST /api/v1/models/:id/deploy`):
```
1. Claim deployment slot (multi-instance: pick healthy instance with capacity)
2. Load LoRA adapter via pluggable backend (vLLM, TGI, or SGLang)
3. Store deployment_status='active' + inference_instance_id in same transaction
4. Enqueue billing event in same transaction (durable outbox)
```

**Inference** (`POST /v1/chat/completions`):
```
1. Client sends request with API key (Bearer pl_sk_...)
2. Rust API verifies key via SHA-256 hash lookup
3. Rate limit check (Redis sliding window)
4. Resolve backend: look up assigned inference instance (or global fallback)
5. Circuit breaker check, forward request with adapter name
6. Stream or return response
7. Bill tokens via durable billing outbox
```

**GGUF Export** (`ExportWorkflow`):
```
1. Download LoRA adapter from S3
2. Merge adapter into base model using peft
3. Convert merged model to GGUF format (llama.cpp)
4. Quantize (Q4_K_M, Q5_K_M, Q6_K, Q8_0)
5. Upload GGUF file to S3
6. Generate presigned download URL for user
```

---

## 7. Infrastructure & Runtime Services

### What Runs in Docker Compose

| Service | Image | Port | Purpose |
|---------|-------|------|---------|
| PostgreSQL 16 | `postgres:16-alpine` | 5432 | Application database (metadata, billing, auth) |
| Redis 7 | `redis:7-alpine` | 6379 | Cache, rate limiting, metrics streaming |
| MinIO | `minio/minio:latest` | 9000 (API), 9001 (UI) | S3-compatible object storage |
| Temporal Server | `temporalio/auto-setup` | 7233 | Workflow orchestration engine |
| Temporal DB | `postgres:16-alpine` | 5433 | Temporal's own PostgreSQL (separate) |
| Temporal UI | `temporalio/ui` | 8088 | Workflow monitoring dashboard |

### What Runs Natively

| Service | Command | Port | Purpose |
|---------|---------|------|---------|
| Rust API | `make dev-api` / `cargo run -p platform-api` | 8000 | HTTP API server |
| Next.js Web | `cd apps/web && pnpm dev` | 3000 | Frontend dev server |
| Python Workers | `cd apps/workers && uv run python -m src.worker` | — | Temporal activity executor |

### Optional Services

| Service | When Needed | Purpose |
|---------|-------------|---------|
| vLLM | For model deployment & inference | GPU inference server with LoRA support |
| OTEL Collector + Grafana | For observability | Distributed tracing and monitoring |

---

## 8. Authentication & Multi-Tenancy

BrainDrain has a **dual authentication system**:

### Platform Users (Clerk JWT)
- Used by the dashboard UI
- Clerk handles sign-in/sign-up/session
- JWT verified by Rust middleware on every `/api/v1/*` request
- **Dev mode:** Accepts `dev_{tenant_uuid}_{user_id}` tokens for local development without Clerk

### Model Consumers (API Keys)
- Used for inference (`/v1/chat/completions`)
- Format: `pl_sk_{random_32_chars}`
- Stored as SHA-256 hash in PostgreSQL (never stored raw)
- Rate-limited via Redis sliding window per key
- Each key is scoped to exactly one model

### Multi-Tenancy Enforcement
- **Application Layer:** Every repository method requires `tenant_id` as a parameter
- **Database Layer:** Row-Level Security (RLS) policies on all tenant-scoped tables using `current_setting('app.tenant_id')`
- **Storage Layer:** S3 paths are tenant-scoped: `{tenant_id}/{project_id}/...`

### RBAC (Role-Based Access Control)
- Roles: `Owner`, `Admin`, `Member`, `Viewer`
- Enforced via `require_role(&user, TeamRole::Admin)?` guard in route handlers
- Team invitations with token-based acceptance flow

### Per-Tenant Configuration
- **Storage:** `tenants.settings` JSONB column (no extra tables or migrations)
- **LLM Provider:** Each tenant can configure their own LLM provider, API key, model, and max tokens via `PUT /api/v1/settings/llm` (admin role required)
- **Security:** API keys stored in DB JSONB, masked in API responses (`sk-p...wxyz`), never appear in Temporal workflow payloads
- **Resolution:** Workers query tenant config from DB at activity execution time → fall back to platform env var defaults if not set
- **Audit:** All settings changes are logged with `api_key_changed: bool` (never the actual key)
- **Extensible:** Same JSONB structure supports future config namespaces (HuggingFace token, vLLM URL, etc.) without migrations

---

## 9. Billing & Usage Metering

### How Billing Works

```
Inference Request
    │
    ▼
┌─────────────────────────────────────────────────────────┐
│  Token Estimator (token_estimator.rs)                   │
│  • Count prompt tokens + completion tokens              │
│  • Calculate cost: $0.15/1M input, $0.60/1M output      │
└──────────────────────┬──────────────────────────────────┘
                       │
                       ▼
┌─────────────────────────────────────────────────────────┐
│  Billing Batcher (billing_batcher.rs)                   │
│  • Non-blocking: event.try_send() via mpsc channel      │
│  • Background worker collects events into Vec           │
│  • Bulk INSERT every 5 seconds or 1000 events           │
│  • Reduces 10K INSERTs/min → 12 bulk INSERTs/min        │
│  • Graceful shutdown flushes remaining events           │
└──────────────────────┬──────────────────────────────────┘
                       │
                       ▼
        PostgreSQL billing_events table
        (partitioned by month)
```

### Stripe Integration
- Raw HTTP integration (no `stripe-rust` crate — faster compile times)
- HMAC-SHA256 webhook verification with constant-time comparison
- Checkout session flow for subscription management
- Webhook handlers: `checkout.session.completed`, `customer.subscription.updated/deleted`

---

## 10. Resilience & Scaling Patterns

### Circuit Breaker (vLLM Protection)

```
                Circuit Breaker State Machine
                ┌──────────────────────────┐
                │                          │
    ┌───────────▼───────────┐   5 failures │
    │       CLOSED          │──────────────┘
    │  (normal operation)   │
    │ Requests pass through │───────────────┐
    └───────────────────────┘               │
                                   5 consecutive
                                   failures
                                            │
                                            ▼
                              ┌───────────────────────┐
                              │        OPEN           │
                              │  (instant 503 reject) │
                              │  No requests to vLLM  │
                              └──────────┬────────────┘
                                         │ 30 seconds
                                         ▼
                              ┌───────────────────────┐
                              │      HALF-OPEN        │
                              │  (1 probe request)    │
                   success ◄──│  Others rejected      │──► failure
                     │        └───────────────────────┘       │
                     │                                        │
                     ▼                                        ▼
                  CLOSED                                    OPEN
```

### Dashboard Caching

```
Dashboard Request → Check Redis → Cache Hit?
                                    │
                         ┌──── YES ─┘──── NO ────┐
                         │                       │
                   Return cached              Run 7 parallel
                   JSON instantly             COUNT queries
                                              (try_join!)
                                                  │
                                                  ▼
                                           Store in Redis
                                           (30s TTL)
                                                  │
                                                  ▼
                                           Return fresh data
```

---

## 11. Flow Diagrams

### Complete System Flow (Everything Connected)

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                            USER (Browser)                                   │
│                                                                             │
│   Dashboard  ←→  Project Page  ←→  Training  ←→  Playground  ←→  Settings   │
└──────────────────────────┬──────────────────────────────────────────────────┘
                           │ HTTP (React Query)
                           ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│                         NEXT.JS WEB (port 3000)                              │
│   Clerk Auth │ React Query Hooks │ SSE Stream Reader │ API Client            │
└──────────────────────────┬───────────────────────────────────────────────────┘
                           │ HTTP + SSE
                           ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│                         RUST API (port 8000)                                 │
│                                                                              │
│  ┌────────────────────────────────────────────────────────────────────────┐  │
│  │  Middleware:  Clerk JWT Verify │ CORS │ Request ID │ Rate Limiting     │  │
│  └────────────────────────────────────────────────────────────────────────┘  │
│                                                                              │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌─────────────────┐     │
│  │ Projects │ │Documents │ │ Training │ │Dashboard │ │   Inference     │     │
│  │  Routes  │ │  Routes  │ │  Routes  │ │  Routes  │ │ /v1/chat/compl. │     │
│  └────┬─────┘ └────┬─────┘ └────┬─────┘ └────┬─────┘ └───────┬─────────┘     │
│       │           │            │             │               │               │
│       ▼           ▼            ▼             ▼               ▼               │
│  ┌──────────────────────────────────────────────────────────────────────┐    │
│  │  Service Layer:  RBAC │ Validation │ Orchestration │ Audit Logging   │    │
│  └──────────────────────────────────────────────────────────────────────┘    │
│       │             │            │             │               │             │
│       ▼             ▼            ▼             ▼               ▼             │
│  ┌────────┐  ┌──────────┐  ┌─────────┐   ┌─────────┐  ┌───────────────┐      │
│  │  Repos │  │ S3/MinIO │  │Temporal │   │  Redis  │  │Circuit Breaker│      │
│  │  (SQL) │  │ Storage  │  │ Client  │   │  Cache  │  │   → vLLM      │      │
│  └───┬────┘  └────┬─────┘  └────┬────┘   └────┬────┘  └──────┬────────┘      │
│      │            │             │             │              │               │
└──────│────────────│─────────────│─────────────│──────────────│───────────────┘
       │            │             │             │              │
       ▼            ▼             ▼             ▼              ▼
  ┌─────────┐  ┌─────────┐  ┌──────────┐  ┌─────────┐  ┌──────────┐
  │Postgres │  │  MinIO  │  │ Temporal │  │  Redis  │  │   vLLM   │
  │  :5432  │  │  :9000  │  │  :7233   │  │  :6379  │  │  :8080   │
  └─────────┘  └─────────┘  └────┬─────┘  └─────────┘  └──────────┘
                                 │
                                 │ Task Queue
                                 ▼
                       ┌──────────────────────┐
                       │   PYTHON WORKERS     │
                       │                      │
                       │  "ml-pipeline" queue │
                       │  ├── parse_document  │
                       │  ├── chunk_text      │
                       │  ├── generate_pairs  │
                       │  ├── build_dataset   │
                       │  └── export_gguf     │
                       │                      │
                       │  "ml-pipeline-gpu"   │
                       │  ├── start_training  │
                       │  └── run_evaluation  │
                       └──────────────────────┘
```

### Individual Pipeline Flow

```
┌──────────────────────────────────────────────────────────────────────┐
│                        INGEST PIPELINE                               │
│                                                                      │
│   POST /parse  ──►  Temporal IngestWorkflow                          │
│                       │                                              │
│                       ├── For each document:                         │
│                       │   ├── 1. get_document_info (fetch metadata)  │
│                       │   └── 2. parse_document (extract text)       │
│                       │          └── Updates DB: status = "parsed"   │
│                       │                                              │
│                       └── Workflow complete → API polled by frontend │
└──────────────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────────────┐
│                        REFINE PIPELINE                               │
│                                                                      │
│   POST /refine ──►  Temporal RefineWorkflow                          │
│                       │                                              │
│                       ├── 1. chunk_text (split parsed docs)          │
│                       ├── 2. generate_pairs (LLM synthesis)          │
│                       └── 3. build_dataset (filter + format)         │
│                              └── Creates dataset in DB + S3          │
└──────────────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────────────┐
│                        TRAINING PIPELINE                             │
│                                                                      │
│   POST /training-jobs ──►  Temporal TrainWorkflow (GPU queue)        │
│                               │                                      │
│                               ├── 1. Provision GPU                   │
│                               ├── 2. Load base model + LoRA config   │
│                               ├── 3. Train (SFT/DPO/GRPO/Iterative)  │
│                               ├── 4. Save adapter to S3              │
│                               └── 5. Create model record in DB       │
└──────────────────────────────────────────────────────────────────────┘
```

---

## 12. Feature Completeness Matrix

### What is Fully Built & Ready to Test

| Feature | Backend | Frontend | Workers | Status |
|---------|---------|----------|---------|--------|
| User authentication (Clerk) | ✅ | ✅ | — | Ready |
| Dev token auth (no Clerk needed) | ✅ | — | — | Ready |
| Project CRUD | ✅ | ✅ | — | Ready |
| Document upload (multipart → S3) | ✅ | ✅ | — | Ready |
| Document parsing | ✅ | ✅ | ✅ | Ready (PyMuPDF/Docling text extraction — no OCR path for scanned/image-only PDFs) |
| Text chunking | ✅ | — | ✅ | Ready |
| Per-tenant LLM provider settings | ✅ | — | ✅ | Ready |
| Synthetic data generation | ✅ | — | ✅ | Ready (needs LLM API key via settings or env var) |
| Dataset building + preview | ✅ | ✅ | ✅ | Ready |
| Training (4 modes) | ✅ | ✅ | ✅ | Ready (needs GPU — local attached GPU is the proven path; Modal cloud GPU is validated for smoke-test/deploy, full train→S3 on cloud is not yet proven end-to-end; no RunPod integration) |
| Evaluation (4 suites) | ✅ | ✅ | ✅ | Ready (needs GPU + LLM key via settings or env var) |
| Model deployment to vLLM | ✅ | ✅ | ✅ | Ready (needs vLLM running; TGI/SGLang also implemented via the backend abstraction but less exercised than vLLM; the serving + automated-CD path is present in code but not proven against sustained production traffic) |
| API key management | ✅ | ✅ | — | Ready |
| Inference proxy (circuit breaker) | ✅ | — | — | Ready (needs vLLM) |
| SSE streaming playground | ✅ | ✅ | — | Ready (needs vLLM) |
| GGUF export | ✅ | ✅ | ✅ | Ready (needs GPU for large models) |
| Billing micro-batcher | ✅ | — | — | Ready |
| Usage dashboard | ✅ | ✅ | — | Ready |
| Team RBAC | ✅ | ✅ | — | Ready |
| Stripe billing | ✅ | ✅ | — | Ready (needs Stripe keys) |
| Dashboard Redis cache | ✅ | — | — | Ready |
| Audit logging | ✅ | — | — | Ready |
| Notification preferences | ✅ | — | — | Ready |
| OpenAPI docs (Swagger UI) | ✅ | — | — | Ready (at `/docs` in non-prod) |
| IP rate limiting | ✅ | — | — | Ready |
| Security headers (CSP, HSTS) | ✅ | — | — | Ready |
| Request ID tracing | ✅ | — | — | Ready |
| HTTP metrics (OTEL) | ✅ | — | — | Ready (optional OTEL export) |

### What Can Be Tested Without a GPU

These core data pipeline features work on CPU and are the first things to test:

1. ✅ Start infrastructure (Docker Compose)
2. ✅ Run migrations
3. ✅ Start Rust API
4. ✅ Start Next.js frontend
5. ✅ Create a project (via UI or API)
6. ✅ Upload a document (PDF, DOCX, TXT)
7. ✅ Start Temporal workers
8. ✅ Trigger document parsing
9. ✅ Trigger data refinement (needs LLM API key — via `PUT /settings/llm` or env var)
10. ✅ Review generated dataset

### What Requires Additional Infrastructure

| Feature | Requires |
|---------|----------|
| Training | GPU (NVIDIA, A10G+ recommended) |
| Evaluation | GPU + LLM API key (via settings or env var) |
| vLLM deployment | NVIDIA GPU + base model downloaded |
| Inference/Playground | Running vLLM server |
| Stripe billing | Stripe test keys |
| Clerk auth (production) | Clerk account + keys |
| GGUF export | GPU for large models, CPU for small |

---

*This document is the single source of truth for understanding BrainDrain. For setup and running instructions, see [QUICKSTART.md](./QUICKSTART.md).*
