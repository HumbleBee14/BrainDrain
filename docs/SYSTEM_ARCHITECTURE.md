# System Architecture

> Complete technical architecture of the LLM fine-tuning platform — services, data flow, pluggable backends, resilience patterns, and configuration hierarchy.

---

## High-Level Service Map

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           LLM FINE-TUNING PLATFORM                          │
│           Upload → Parse → Refine → Train → Evaluate → Deploy               │
└─────────────────────────────────────────────────────────────────────────────┘

┌──────────────┐     ┌──────────────────────────────────────────────────────┐
│   Frontend   │     │                   Rust API (Axum)                    │
│  Next.js 15  │────▶│  Route → Service → Repository (multi-tenant)        │
│              │◀────│                                                      │
│  - Dashboard │     │  Auth: Clerk JWT + API Key + Dev Token               │
│  - Projects  │ SSE │  Storage: S3 trait (MinIO / AWS)                     │
│  - Training  │◀────│  Cache: Redis (pub/sub + streams)                    │
│  - Settings  │     │  Queue: Temporal client                              │
│  - Deploy    │     │  Billing: Usage batcher                              │
│  - Inference │     │  Notifications: Webhook + Email                      │
└──────────────┘     └──────────┬───────────────┬───────────────────────────┘
                                │               │
                     Temporal   │               │  Direct
                     Workflows  │               │  Queries
                                ▼               ▼
┌───────────────────────────────────────┐  ┌─────────────┐  ┌─────────────┐
│         Python Workers (Temporal)      │  │ PostgreSQL  │  │    Redis    │
│                                        │  │             │  │             │
│  10 Pluggable Backends                 │  │ - Tenants   │  │ - SSE push  │
│  Full Pipeline Workflows               │  │ - Projects  │  │ - Metrics   │
│  Circuit Breaker + Heartbeats          │  │ - Documents │  │   streams   │
│                                        │  │ - Jobs      │  │ - Cache     │
│  Worker Modes:                         │  │ - Models    │  │ - Pub/Sub   │
│    all  — single process               │  │ - Evals     │  │             │
│    main — CPU-only activities          │  │ - Settings  │  └─────────────┘
│    gpu  — training + inference only    │  │   (JSONB)   │
│                                        │  │             │  ┌─────────────┐
│  GPU Providers:                        │  │ RLS per     │  │  S3 / MinIO │
│    local — worker's own GPU            │  │ tenant_id   │  │             │
│    modal — serverless (Modal.com)      │  └─────────────┘  │ - Documents │
└────────────────────────────────────────┘                    │ - Chunks    │
                                                              │ - Pairs     │
                                                              │ - Datasets  │
                                                              │ - Adapters  │
                                                              │ - Exports   │
                                                              └─────────────┘
```

---

## Core Pipeline Flow

```
                          USER UPLOADS DOCUMENTS
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│                         INGEST WORKFLOW                              │
│                                                                     │
│   For each document:                                                │
│   ┌──────────┐    ┌──────────────┐    ┌───────────────────┐        │
│   │  Fetch   │───▶│  PDF Parse   │───▶│  Language Detect  │        │
│   │  from S3 │    │  (pluggable) │    │  (pluggable)      │        │
│   └──────────┘    └──────────────┘    └────────┬──────────┘        │
│                                                 │                   │
│                    pymupdf (default, fast)       │  langdetect      │
│                    docling (richer structure)    │  null (disable)  │
│                                                 ▼                   │
│                                        Store parsed text in S3     │
└─────────────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│                         REFINE WORKFLOW                              │
│                                                                     │
│   ┌──────────────┐   ┌─────────────────┐   ┌──────────────────┐   │
│   │  Chunk Text  │──▶│  Generate Pairs  │──▶│  Build Dataset   │   │
│   │  (pluggable) │   │  (pluggable LLM) │   │  (filter + dedup)│   │
│   └──────────────┘   └─────────────────┘   └──────────────────┘   │
│                                                                     │
│   Chunking:              LLM Provider:          Filter:             │
│     recursive (default)    openai-compat          heuristic         │
│     sliding (fixed)        (any OpenAI API)                         │
│                                                 Dedup:              │
│   Chunk → prompt LLM    Prompt types:             hash (MD5)        │
│   → Q&A / instruction   question_answering                          │
│   / reasoning pairs      instruction_following    Output: ChatML    │
│                          reasoning                JSONL dataset     │
└─────────────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│                         TRAIN WORKFLOW                               │
│                                                                     │
│   Mode dispatcher routes to the appropriate strategy:               │
│                                                                     │
│   ┌──────────┐  ┌────────────┐  ┌───────────┐  ┌──────────────┐   │
│   │  quick   │  │ iterative  │  │  aligned  │  │  reasoning   │   │
│   │          │  │            │  │           │  │              │   │
│   │ Single   │  │ SFT loop + │  │ SFT → DPO │  │ SFT → GRPO  │   │
│   │ SFT run  │  │ early stop │  │ (human    │  │ (reward-     │   │
│   │          │  │ + holdout  │  │  align)   │  │  guided)     │   │
│   └──────────┘  └────────────┘  └───────────┘  └──────────────┘   │
│                                                                     │
│   Training Engine: unsloth (default, fast LoRA/QLoRA)               │
│   Metrics Sink:    redis (real-time) | log | null                   │
│                                                                     │
│   Features:                                                         │
│   - Real-time loss/ETA streaming to dashboard via Redis Streams     │
│   - Checkpoint upload to S3 during training                         │
│   - GPU metrics collection (utilization, memory, temperature)       │
│   - Configurable hyperparameters (lr, epochs, rank, batch size)     │
└─────────────────────────────────────────────────────────────────────┘
                                   │
                          ┌────────┴────────┐
                          ▼                 ▼
┌──────────────────────────────┐  ┌──────────────────────────────────┐
│       EVALUATE WORKFLOW       │  │         EXPORT WORKFLOW           │
│                               │  │                                  │
│  ┌─────────────────────────┐ │  │  ┌────────────────────────────┐  │
│  │   LLM Judge (pluggable) │ │  │  │  GGUF Quantize + Upload   │  │
│  │                         │ │  │  │                            │  │
│  │  - Accuracy scoring     │ │  │  │  Formats: Q4_K_M, Q5_K_M, │  │
│  │  - Relevance scoring    │ │  │  │  Q8_0, F16, F32           │  │
│  │  - Faithfulness scoring │ │  │  │                            │  │
│  │  - Safety benchmarks    │ │  │  │  Upload to S3 + optional   │  │
│  │  - Composite score      │ │  │  │  HuggingFace Hub push     │  │
│  └─────────────────────────┘ │  │  └────────────────────────────┘  │
│                               │  │                                  │
│  Judge Backend:               │  │  Model Loader: unsloth           │
│    openai (any compatible)    │  │                                  │
└──────────────────────────────┘  └──────────────────────────────────┘
                          │                 │
                          └────────┬────────┘
                                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│                         DEPLOY (vLLM)                               │
│                                                                     │
│   Load LoRA adapter into vLLM server                                │
│   Serve via OpenAI-compatible inference API                         │
│                                                                     │
│   Endpoints:                                                        │
│     POST /v1/chat/completions        (single, streaming)            │
│     POST /v1/chat/completions/batch  (batch, concurrent)            │
│                                                                     │
│   Auth: per-model API keys (scoped to tenant + model)               │
│   Billing: token-based usage metering (batched writes)              │
│   Resilience: circuit breaker on vLLM calls                         │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Pluggable Backend Architecture

Every processing layer follows the same pattern: **Protocol (interface) → Implementations → Registry → Factory**.

```
┌─────────────────────────────────────────────────────────────────────┐
│                    BACKEND ABSTRACTION PATTERN                       │
│                                                                     │
│   class MyBackend(Protocol):         # Interface contract           │
│       def process(self, ...) -> T    # What every impl must do      │
│                                                                     │
│   class ConcreteImpl:                # One or more implementations  │
│       def process(self, ...) -> T    # Actual logic                 │
│                                                                     │
│   _REGISTRY = {"name": ConcreteImpl} # Name → class mapping         │
│                                                                     │
│   register(name, cls)                # Add custom implementations   │
│   get(name) -> MyBackend             # Factory: instantiate by name │
│                                                                     │
│   Selected via: APP_*_BACKEND env var in WorkerSettings             │
│   Default: always the current production implementation             │
│   Swap: change one env var, zero code changes                       │
└─────────────────────────────────────────────────────────────────────┘
```

### All 10 Pluggable Backends

| Backend | ENV Var | Default | Alternatives | File |
|---------|---------|---------|-------------|------|
| PDF extraction | `APP_PDF_BACKEND` | `pymupdf` | `docling` | `backends/pdf_extractor.py` |
| Language detection | `APP_LANGUAGE_DETECTOR_BACKEND` | `langdetect` | `null` | `backends/language_detector.py` |
| Text chunking | `APP_CHUNKING_BACKEND` | `recursive` | `sliding` | `backends/chunking_strategy.py` |
| LLM provider | `APP_LLM_PROVIDER_BACKEND` | `openai` | any OpenAI-compat | `backends/llm_provider.py` |
| Dataset filter | `APP_DATASET_FILTER_BACKEND` | `heuristic` | (extensible) | `backends/dataset_filter.py` |
| Deduplication | `APP_DEDUP_BACKEND` | `hash` | (extensible) | `backends/dataset_filter.py` |
| LLM judge | `APP_JUDGE_BACKEND` | `openai` | any OpenAI-compat | `backends/judge.py` |
| Training engine | `APP_TRAINING_ENGINE` | `unsloth` | (extensible) | `activities/training_engine.py` |
| Metrics collector | `APP_METRICS_BACKEND` | `redis` | `log`, `null` | `backends/metrics_collector.py` |
| Model inference | `APP_EVAL_MODEL_LOADER` | `unsloth` | (extensible) | `backends/model_inference.py` |

---

## Configuration Hierarchy

Tenant-specific settings override platform defaults. No migration needed — uses existing `tenants.settings` JSONB column.

```
┌─────────────────────────────────────────────────────────────────────┐
│                     CONFIGURATION RESOLUTION ORDER                   │
│                                                                     │
│   1. Per-Tenant DB Settings     (highest priority)                  │
│      tenants.settings JSONB                                         │
│      Managed via: Settings > LLM Provider in UI                     │
│      API: PUT /api/v1/settings/llm                                  │
│                                                                     │
│   2. Worker Environment Vars    (platform-wide defaults)            │
│      APP_LLM_API_BASE_URL, APP_LLM_API_KEY, APP_LLM_MODEL, etc.   │
│      Set in .env or docker-compose                                  │
│                                                                     │
│   3. Hardcoded Defaults         (lowest priority)                   │
│      config.py / config.rs field defaults                           │
│      Only used if nothing else is set                               │
└─────────────────────────────────────────────────────────────────────┘

Settings JSONB structure:
{
  "llm": {
    "provider": "openai",
    "api_base_url": "https://api.openai.com/v1",
    "api_key": "sk-proj-...",           // masked in API responses
    "model": "gpt-4o-mini",
    "max_tokens": 2000
  }
}
```

---

## Multi-Tenancy & Isolation

```
┌─────────────────────────────────────────────────────────────────────┐
│                     MULTI-TENANCY ENFORCEMENT                       │
│                                                                     │
│   Layer 1: Application Level                                        │
│   ├── Every repository query includes WHERE tenant_id = $1          │
│   ├── Auth middleware extracts tenant_id from JWT claims             │
│   └── API keys are scoped to (tenant_id, model_id)                  │
│                                                                     │
│   Layer 2: Database Level (RLS)                                     │
│   ├── Row-Level Security policies on all tenant-scoped tables       │
│   ├── SET LOCAL app.tenant_id = $1 per transaction                  │
│   ├── before_acquire hook resets tenant_id on connection checkout    │
│   └── Belt + suspenders: app-level + DB-level isolation             │
│                                                                     │
│   Layer 3: Storage Level                                            │
│   ├── S3 paths namespaced: tenants/{tenant_id}/projects/{id}/...    │
│   └── No cross-tenant path traversal possible                       │
│                                                                     │
│   Layer 4: Workflow Level                                           │
│   ├── tenant_id propagated in every Temporal workflow input          │
│   └── Workers resolve per-tenant config at activity execution time  │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Resilience Patterns

```
┌─────────────────────────────────────────────────────────────────────┐
│                       RESILIENCE & RELIABILITY                      │
│                                                                     │
│   Circuit Breaker                                                   │
│   ├── LLM API calls (generation, judging)                           │
│   ├── vLLM inference calls                                          │
│   ├── Configurable: fail_max, reset_timeout                         │
│   └── Prevents cascading failures from downstream outages           │
│                                                                     │
│   Temporal Durable Execution                                        │
│   ├── All pipeline stages are Temporal activities                   │
│   ├── Automatic retry with configurable backoff                     │
│   ├── Workflow state survives worker crashes                         │
│   ├── Configurable timeouts per activity type                       │
│   └── Heartbeats for long-running activities (training, generation) │
│                                                                     │
│   Real-Time Streaming                                               │
│   ├── SSE push from API → frontend (no polling)                     │
│   ├── Redis Streams for training metrics                            │
│   ├── WebSocket bridge for live metric updates                      │
│   └── Graceful degradation if Redis unavailable                     │
│                                                                     │
│   Graceful Shutdown                                                 │
│   ├── SIGTERM triggers coordinated shutdown                         │
│   ├── Billing batcher flushes pending writes                        │
│   ├── Notification worker completes in-flight deliveries            │
│   └── Connection pools drained cleanly                              │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Security Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                         SECURITY LAYERS                             │
│                                                                     │
│   Authentication                                                    │
│   ├── Clerk JWT verification (frontend sessions)                    │
│   ├── API key auth (inference endpoints, scoped per model)          │
│   └── Dev token (development only, disabled in production)          │
│                                                                     │
│   Authorization                                                     │
│   ├── Team-based RBAC: Owner > Admin > Member > Viewer              │
│   ├── Route-level role checks via extractors                        │
│   └── Admin-only: settings, team management, deployments            │
│                                                                     │
│   API Security                                                      │
│   ├── Rate limiting: per-IP and per-API-key                         │
│   ├── CORS: configurable origins (no wildcard in production)        │
│   ├── Security headers: CSP, HSTS, X-Frame-Options, X-Content-Type │
│   └── Request ID tracing: distributed across API + workers          │
│                                                                     │
│   Data Security                                                     │
│   ├── API keys masked in responses (sk-p...wxyz)                    │
│   ├── Secrets never pass through Temporal (resolved at runtime)     │
│   ├── SSRF protection on webhooks (private IP filtering)            │
│   └── SQL injection prevented via parameterized queries (SQLx)      │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Rust API Layer Detail

```
┌─────────────────────────────────────────────────────────────────────┐
│                    RUST API — LAYERED ARCHITECTURE                   │
│                                                                     │
│   Routes (thin)                                                     │
│   ├── Extract auth (JWT / API key)                                  │
│   ├── Validate request DTO                                          │
│   ├── Call service                                                  │
│   └── Return response DTO                                           │
│        │                                                            │
│        ▼                                                            │
│   Services (business logic)                                         │
│   ├── Orchestrate repositories, S3, Redis                           │
│   ├── Validation rules, state transitions                           │
│   ├── Trigger Temporal workflows                                    │
│   └── Never hold full AppState — only specific dependencies         │
│        │                                                            │
│        ▼                                                            │
│   Repositories (data access)                                        │
│   ├── Pure SQL via SQLx (compile-time checked)                      │
│   ├── Every query includes tenant_id                                │
│   ├── Trait-based (mockable for tests)                              │
│   └── Return domain models, not DTOs                                │
│                                                                     │
│   Type Flow:                                                        │
│   Rust DTO (#[derive(TS)]) → cargo test → .ts files → Frontend     │
│   Single source of truth: Rust types generate TypeScript types      │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Deployment Topology

```
┌─────────────────────────────────────────────────────────────────────┐
│                     PRODUCTION DEPLOYMENT                            │
│                                                                     │
│   ┌─────────┐  ┌─────────┐  ┌──────────┐  ┌─────────────────┐    │
│   │  Web    │  │  API    │  │ Workers  │  │  Workers (GPU)  │    │
│   │ Next.js │  │  Rust   │  │  Python  │  │  Python         │    │
│   │         │  │  Axum   │  │  CPU-only │  │  training +     │    │
│   │ :3000   │  │  :8000  │  │          │  │  inference      │    │
│   └────┬────┘  └────┬────┘  └────┬─────┘  └────────┬────────┘    │
│        │            │            │                   │             │
│        └────────────┼────────────┼───────────────────┘             │
│                     │            │                                  │
│   ┌─────────────────┼────────────┼──────────────────────────┐     │
│   │  Infrastructure  │            │                          │     │
│   │                  ▼            ▼                          │     │
│   │  ┌──────────┐  ┌──────────┐  ┌──────────┐              │     │
│   │  │PostgreSQL│  │  Redis   │  │ Temporal │              │     │
│   │  │  + RLS   │  │          │  │ Server + │              │     │
│   │  │          │  │          │  │   DB     │              │     │
│   │  └──────────┘  └──────────┘  └──────────┘              │     │
│   │                                                         │     │
│   │  ┌──────────┐  ┌──────────┐                            │     │
│   │  │ S3/MinIO │  │  vLLM    │                            │     │
│   │  │          │  │  Server  │                            │     │
│   │  └──────────┘  └──────────┘                            │     │
│   └─────────────────────────────────────────────────────────┘     │
└─────────────────────────────────────────────────────────────────────┘

Container images:
  platform-api:latest     — Rust binary (~15MB, <100ms cold start)
  platform-workers:latest — Python + ML libs (~4GB with CUDA)
  platform-web:latest     — Next.js standalone (~100MB)
```
