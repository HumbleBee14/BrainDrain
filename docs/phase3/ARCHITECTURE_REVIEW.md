# Architecture Review — Post-Phase 3 Codebase Audit

> Senior architect-level review of the full codebase after completing Phases 0-3. Identifies structural gaps, SOLID violations, tight coupling points, and extensibility blockers. These are known gaps to address in future sprints.

**Review Date:** 2026-02-21
**Codebase State:** Phases 0-3 complete. Upload → Parse → Refine → Train → Evaluate → Deploy → Infer all functional.
**Overall Score: 7.2/10**

---

## Table of Contents

1. [What's Done Well](#whats-done-well)
2. [Critical Issues (P0)](#critical-issues-p0)
3. [High-Priority Issues (P1)](#high-priority-issues-p1)
4. [Medium-Priority Issues (P2)](#medium-priority-issues-p2)
5. [Low-Priority Issues (P3)](#low-priority-issues-p3)
6. [SOLID Principles Assessment](#solid-principles-assessment)
7. [Extensibility Scorecard](#extensibility-scorecard)
8. [Layer-by-Layer Scores](#layer-by-layer-scores)
9. [Recommended Refactoring Order](#recommended-refactoring-order)
10. [Final Verdict](#final-verdict)

---

## What's Done Well

These are architectural strengths to preserve. Do not regress on these.

| Strength | Evidence | Layer |
|---|---|---|
| **Multi-tenancy enforcement** | Every SQL query includes `tenant_id` in WHERE clause. Zero exceptions across 30+ repository methods. API key hash lookup is the only intentional exception (it returns `tenant_id`). | Rust |
| **Route → Service → Repository** | Consistently applied across all domains. Routes are thin (extract auth, parse request, return JSON). Services contain business logic. Repos are pure SQL. No violations found. | Rust |
| **ObjectStorage trait** | Clean `impl ObjectStorage` abstraction in `crates/storage/src/lib.rs`. Services depend on the trait, not concrete `S3Storage`. Could swap to GCS/Azure Blob/MinIO without touching service code. | Rust |
| **Workflows are thin orchestrators** | Temporal workflows (train, evaluate, refine, ingest) contain zero business logic — purely sequence activities with timeouts and retry policies. Textbook Temporal usage. | Python |
| **Config management** | Pydantic `BaseSettings` (Python) and env-based `Config` (Rust) with sensible defaults. No hardcoded brand names anywhere. Generic names (`platform-api`, `platform-db`). | Both |
| **Error handling** | `AppError` enum doesn't leak internal details. Consistent JSON envelope `{"error":{"code":"...","message":"..."}}`. Proper `From<>` implementations for SQLx, storage, and anyhow errors. | Rust |
| **Shared enums** | `strum` for Rust (Display + EnumString), mirrored in TypeScript. Consistent `snake_case` wire format across languages. | Cross-cutting |
| **S3 path builders** | Identical path functions in Rust (`crates/shared/src/s3_paths.rs`) and Python (`apps/workers/src/s3_paths.py`). Tenant-scoped paths prevent cross-tenant data access. | Both |
| **API key security** | SHA-256 hashing, key-shown-once pattern, Redis per-minute rate limiting, expiry support, soft-delete revocation. Matches industry standard (Stripe, OpenAI). | Rust |
| **Parallel queries** | `tokio::try_join!` used consistently for independent DB operations. Pipeline status runs 21 parallel COUNT queries. List endpoints do parallel fetch + count. | Rust |
| **GPU queue separation** | CPU activities (parse, chunk, generate) on `ml-pipeline-main` queue. GPU activities (train, evaluate) on `ml-pipeline-gpu` queue. Clean worker mode routing (dev/main/gpu). | Python |
| **Dual authentication** | Clerk JWT for platform users and API key auth for model consumers are completely separate Axum extractors on separate route trees. No coupling between the two. | Rust |
| **Graceful degradation** | API runs without Temporal (`Option<TemporalClient>`). LLM judge falls back to heuristics. vLLM unavailability doesn't crash deployment service. | Both |

---

## Critical Issues (P0)

These are blockers for production scalability and ML library evolution. Should be addressed before or during next phase.

### P0-1: No Trait Abstraction for TemporalClient

**Layer:** Rust API
**Impact:** Cannot swap workflow engines. Cannot mock in tests. Cannot test service layer in isolation.

**Current State:**
```rust
// crates/api/src/services/training_job_service.rs
pub async fn create(
    db: &PgPool,
    temporal: Option<&TemporalClient>,  // ← Concrete type, not a trait
) -> AppResult<TrainingJobResponse> {
    let temporal = temporal.ok_or(AppError::BadRequest { ... })?;
    let result = temporal.start_train(...).await?;  // ← Bound to Temporal forever
}
```

`TemporalClient` is a concrete struct with HTTP implementation details (reqwest). Every service that triggers workflows takes `Option<&TemporalClient>` directly. This pattern repeats in:
- `TrainingJobService::create()`
- `PipelineService::trigger_parse()`
- `PipelineService::trigger_refine()`
- `EvaluationService::create()`

**What Breaks:**
- Swapping to Apache Airflow, Prefect, or embedded orchestration
- Mocking for unit tests (no tests exist for workflow integration)
- Using Temporal's gRPC client instead of HTTP

**Required Fix:**
```rust
#[async_trait]
pub trait WorkflowOrchestrator: Send + Sync {
    async fn start_train(&self, ...) -> Result<StartWorkflowResponse, OrchestratorError>;
    async fn start_evaluate(&self, ...) -> Result<StartWorkflowResponse, OrchestratorError>;
    async fn start_ingest(&self, ...) -> Result<StartWorkflowResponse, OrchestratorError>;
    async fn start_refine(&self, ...) -> Result<StartWorkflowResponse, OrchestratorError>;
    async fn get_status(&self, workflow_id: &str) -> Result<WorkflowStatus, OrchestratorError>;
}

impl WorkflowOrchestrator for TemporalClient { ... }  // Existing code moves here

// Services take trait reference:
pub async fn create(
    db: &PgPool,
    orchestrator: Option<&dyn WorkflowOrchestrator>,  // ← Trait, swappable
) -> AppResult<TrainingJobResponse>
```

**Effort:** 1-2 days

---

### P0-2: No Training Engine Abstraction

**Layer:** Python Worker
**Impact:** Swapping Unsloth for Axolotl, torchtune, or any new framework requires rewriting 300+ lines across `train_model.py` and `run_evaluation.py`. In a domain where new ML frameworks ship monthly, this is a serious liability.

**Current State:**
```python
# apps/workers/src/activities/train_model.py — Hardwired everywhere
from unsloth import FastLanguageModel
model, tokenizer = FastLanguageModel.from_pretrained(...)  # Can't swap
from trl import SFTTrainer, SFTConfig                      # Can't swap
trainer = SFTTrainer(model=model, ...)                      # Can't swap

# apps/workers/src/activities/run_evaluation.py — Same coupling
from unsloth import FastLanguageModel
model_ft, tokenizer = FastLanguageModel.from_pretrained(...)
from peft import PeftModel
model_ft = PeftModel.from_pretrained(model_ft, str(adapter_local))
FastLanguageModel.for_inference(model_ft)
```

**What Breaks:**
- Swapping to Axolotl (different config format, different trainer API)
- Swapping to torchtune (completely different model loading)
- Using vLLM for evaluation inference (different `generate()` API)
- Any new framework that doesn't follow HuggingFace conventions

**Required Fix:**
```python
class TrainingEngine(Protocol):
    def load_model(self, model_name: str, **kwargs) -> tuple[Any, Any]:
        """Load model + tokenizer."""
        ...

    def get_peft_model(self, model, r: int, alpha: int, **kwargs):
        """Apply LoRA/QLoRA adapter."""
        ...

    def create_sft_trainer(self, model, tokenizer, dataset, config, callbacks) -> Any:
        """Create SFT trainer for this framework."""
        ...

    def create_dpo_trainer(self, model, tokenizer, dataset, config, callbacks) -> Any:
        """Create DPO trainer for this framework."""
        ...

    def prepare_for_inference(self, model) -> None:
        """Prepare model for inference (disable training mode)."""
        ...

class UnslothEngine(TrainingEngine):
    def load_model(self, model_name, **kwargs):
        from unsloth import FastLanguageModel
        return FastLanguageModel.from_pretrained(model_name, ...)

    def create_sft_trainer(self, model, tokenizer, dataset, config, callbacks):
        from trl import SFTTrainer, SFTConfig
        args = SFTConfig(**config)
        return SFTTrainer(model=model, processing_class=tokenizer, ...)

class AxolotlEngine(TrainingEngine):
    # New engine = new file, zero modifications to existing code
    ...
```

**Effort:** 2-3 days

---

### P0-3: No Training Strategy Pattern

**Layer:** Python Worker
**Impact:** Adding a new training mode (ORPO, KTO, SimPO, DPO+GRPO hybrid) requires modifying the core if-elif chain in `_run_training()`. Violates Open/Closed Principle.

**Current State:**
```python
# apps/workers/src/activities/train_model.py — Monolithic dispatch
def _run_training(input, model, tokenizer, dataset, hp, max_seq_length):
    if input.mode == "quick":
        metrics = _train_sft(...)
    elif input.mode == "aligned":
        metrics_sft = _train_sft(...)
        metrics_dpo = _train_dpo(...)
        metrics = {**metrics_sft, "dpo": metrics_dpo}
    elif input.mode == "reasoning":
        metrics_sft = _train_sft(...)
        metrics_grpo = _train_grpo(...)
        metrics = {**metrics_sft, "grpo": metrics_grpo}
    elif input.mode == "iterative":
        metrics = _train_iterative(...)
    else:
        raise ValueError(f"Unknown training mode: {input.mode}")
```

**What Breaks:**
- Adding ORPO requires modifying this function
- Adding KTO requires modifying this function
- Adding any composite mode (SFT → DPO → GRPO) requires modifying this function
- Each modification risks breaking existing modes

**Required Fix:**
```python
class TrainingStrategy(Protocol):
    async def train(
        self, model, tokenizer, dataset, hyperparams: dict, context: TrainingContext,
    ) -> dict:
        """Execute training and return metrics."""
        ...

class SFTStrategy(TrainingStrategy): ...
class AlignedStrategy(TrainingStrategy):  # Composes SFT + DPO
    ...
class ReasoningStrategy(TrainingStrategy):  # Composes SFT + GRPO
    ...

# Registry — Open for extension, closed for modification
TRAINING_STRATEGIES: dict[str, TrainingStrategy] = {
    "quick": SFTStrategy(),
    "aligned": AlignedStrategy(),
    "reasoning": ReasoningStrategy(),
    "iterative": IterativeStrategy(),
}

def register_strategy(name: str, strategy: TrainingStrategy):
    TRAINING_STRATEGIES[name] = strategy

# Adding ORPO = one new file, one register_strategy() call
# Zero modifications to existing code
```

**Effort:** 2-3 days

---

### P0-4: No Evaluation Suite Registry

**Layer:** Python Worker
**Impact:** Adding a 5th evaluation suite (Bias Detection, Robustness, Toxicity) requires modifying `_run_all_suites()`, `_compute_overall()`, and `_generate_recommendations()` in `run_evaluation.py`.

**Current State:**
```python
# apps/workers/src/activities/run_evaluation.py — Hardcoded suite orchestration
async def _run_all_suites(input):
    domain_scores, domain_report = _suite_domain(...)      # Suite 1
    general_scores, general_report = _suite_general(...)    # Suite 2
    ab_scores, ab_report = _suite_ab_comparison(...)        # Suite 3
    safety_scores, safety_report = _suite_safety(...)       # Suite 4
    overall = _compute_overall(domain_scores, general_scores, ab_scores, safety_scores)
```

**Required Fix:**
```python
class EvaluationSuite(Protocol):
    name: str
    weight: float  # Weight in overall score

    async def run(self, model_ft, model_base, tokenizer, val_data, judge) -> tuple[dict, dict]:
        """Return (scores_dict, report_dict)."""
        ...

EVALUATION_SUITES: list[EvaluationSuite] = [
    DomainSuite(weight=0.30),
    GeneralCapabilitySuite(weight=0.25),
    ABComparisonSuite(weight=0.25),
    SafetySuite(weight=0.20),
]

# Adding Bias Detection = one new file, append to list
# Zero modifications to existing suites
```

**Effort:** 1-2 days

---

## High-Priority Issues (P1)

Should be addressed in the next 1-2 sprints.

### P1-1: Status Strings Instead of Enums in DB Models

**Layer:** Rust (crates/db/src/models.rs)
**Impact:** No compile-time type safety. Invalid states are possible. String comparisons throughout services.

```rust
// CURRENT — Stringly typed
pub struct Document { pub status: String }       // Could be "invalid_garbage"
pub struct TrainingJob { pub status: String }     // No IDE autocomplete
pub struct Evaluation { pub status: String }      // No exhaustive match

// ALREADY EXISTS — But not used!
// crates/shared/src/enums.rs
pub enum DocumentStatus { Uploaded, Parsing, Parsed, Failed }
pub enum TrainingJobStatus { Pending, Training, Completed, Failed, Cancelled }
pub enum EvaluationStatus { Running, Completed, Failed }
```

**Fix:** Use the shared enums in DB models. SQLx supports custom type mappings via `sqlx::Type`.

**Effort:** 1 day

---

### P1-2: PostgreSQL RLS Enabled But Not Configured

**Layer:** Database (crates/db/src/migrations/001_initial_schema.sql)
**Impact:** Row Level Security is ENABLED on all tables but has **zero policies**. This is defense-in-depth — if any repository query accidentally omits `tenant_id`, data leaks cross-tenant. Currently the only protection is code-level discipline.

```sql
-- CURRENT: Enabled but toothless
ALTER TABLE projects ENABLE ROW LEVEL SECURITY;
ALTER TABLE documents ENABLE ROW LEVEL SECURITY;
ALTER TABLE datasets ENABLE ROW LEVEL SECURITY;
-- ... all tables
-- NO POLICIES DEFINED!
```

**Fix:**
```sql
-- Set tenant context per request (called from Rust middleware)
SET app.tenant_id = 'uuid-here';

-- Policy enforces tenant isolation at DB level
CREATE POLICY tenant_isolation ON projects
  FOR ALL USING (tenant_id = current_setting('app.tenant_id')::uuid);

CREATE POLICY tenant_isolation ON documents
  FOR ALL USING (tenant_id = current_setting('app.tenant_id')::uuid);
-- ... repeat for all tables
```

**Effort:** 0.5 day

---

### P1-3: Missing Database Indexes for Common Queries

**Layer:** Database
**Impact:** At scale (10K+ tenants, 1M+ documents), these queries will full-scan.

| Missing Index | Used By | Query Pattern |
|---|---|---|
| `documents(project_id, status)` | Pipeline status, document filtering | `WHERE project_id = $1 AND status = $2` |
| `evaluations(model_id, created_at DESC)` | Evaluation listing | `ORDER BY created_at DESC LIMIT $1` |
| `billing_events(tenant_id, created_at DESC)` | Billing history | `WHERE tenant_id = $1 ORDER BY created_at DESC` |
| `api_keys(key_hash)` | API key authentication | `WHERE key_hash = $1 AND is_active = TRUE` |
| `training_jobs(project_id, status)` | Pipeline status | `WHERE project_id = $1 AND status = $2` |

**Effort:** 0.5 day (single migration file)

---

### P1-4: Global Module State in Python Workers

**Layer:** Python Worker (clients.py, train_model.py)
**Impact:** Activities reach for globals (`clients.get_s3()`, `clients.get_db()`, `_get_sync_redis()`, `_get_llm_client()`). Cannot test without full infrastructure. Cannot run tests in parallel. Cannot swap implementations per-test.

```python
# CURRENT: Service locator anti-pattern
_s3_client: boto3.client | None = None
_db_pool: asyncpg.Pool | None = None
_redis: aioredis.Redis | None = None

def get_s3():
    if _s3_client is None:
        raise RuntimeError("Clients not initialized")
    return _s3_client

# Activities reach for globals:
async def parse_document(input):
    s3 = clients.get_s3()       # Implicit dependency
    db = await clients.get_db() # Implicit dependency
```

**Fix:** Protocol-based dependency injection:
```python
class ObjectStorage(Protocol):
    def get_object(self, Bucket: str, Key: str) -> dict: ...
    def put_object(self, Bucket: str, Key: str, Body: bytes) -> None: ...

class Database(Protocol):
    async def fetchrow(self, query: str, *args) -> dict: ...
    async def execute(self, query: str, *args) -> None: ...

class MetricsSink(Protocol):
    def emit(self, job_id: str, metrics: dict) -> None: ...
```

**Effort:** 2-3 days

---

### P1-5: AppState Holds Concrete Types

**Layer:** Rust API (crates/api/src/app_state.rs)
**Impact:** Cannot swap infrastructure (cache backend, workflow engine) without modifying AppState. Testing requires full infrastructure.

```rust
// CURRENT: Concrete types
struct AppStateInner {
    pub config: Config,
    pub db: PgPool,
    pub redis: redis::aio::ConnectionManager,  // ← Concrete
    pub storage: S3Storage,                     // ← Concrete (despite trait existing)
    pub temporal: Option<TemporalClient>,       // ← Concrete
}
```

Note: `ObjectStorage` trait exists but AppState wraps concrete `S3Storage` instead of `Arc<dyn ObjectStorage>`.

**Fix:**
```rust
struct AppStateInner {
    pub config: Config,
    pub db: PgPool,
    pub cache: Arc<dyn CacheBackend>,
    pub storage: Arc<dyn ObjectStorage>,
    pub orchestrator: Option<Arc<dyn WorkflowOrchestrator>>,
}
```

**Effort:** 1-2 days

---

## Medium-Priority Issues (P2)

Address during regular development. Not blockers but add friction.

### P2-1: Repositories Not Trait-Based (Rust)

**Impact:** Cannot mock DB for unit tests. Services calling `ProjectRepo::list(db, ...)` are coupled to concrete SQL.

```rust
// CURRENT: Static methods on marker structs
pub struct ProjectRepo;
impl ProjectRepo {
    pub async fn list(db: &PgPool, ...) -> Result<Vec<Project>, AppError> {
        sqlx::query_as::<_, Project>("SELECT * FROM ...").fetch_all(db).await?
    }
}
```

**Fix (optional):** For unit testing, wrap repos behind traits. However, the current pattern of integration-testing against a real DB is also valid for this stage.

**Effort:** 2-3 days

---

### P2-2: Monolithic Frontend Page Components

**Impact:** `projects/[id]/page.tsx` is 628 lines with 5 inline sub-components (`StatusBadge`, `DocStatusBadge`, `TrainingStatusBadge`, `PipelineStageCard`, `DocumentRow`). Hard to maintain, test, or reuse.

**Fix:** Extract to `components/` folder:
```
components/
├── badges/
│   ├── StatusBadge.tsx
│   ├── DocStatusBadge.tsx
│   └── TrainingStatusBadge.tsx
├── pipeline/
│   └── PipelineStageCard.tsx
├── documents/
│   └── DocumentRow.tsx
└── training/
    └── TrainingForm.tsx
```

**Effort:** 1-2 days

---

### P2-3: Hook Boilerplate Duplication (Frontend)

**Impact:** Same `getToken()` pattern repeated 20+ times across all hooks. DRY violation.

```typescript
// REPEATED IN EVERY HOOK:
const { getToken } = useAuth();
return useQuery({
    queryFn: async () => {
        const token = await getToken();
        if (!token) throw new Error("Not authenticated");
        return api.projects.list(token, offset, limit);
    },
});
```

**Fix:** Hook factory:
```typescript
function useAuthedQuery<T>(
    queryKey: unknown[],
    queryFn: (token: string) => Promise<T>,
    options?: Omit<UseQueryOptions<T>, 'queryKey' | 'queryFn'>
) {
    const { getToken } = useAuth();
    return useQuery<T>({
        queryKey,
        queryFn: async () => {
            const token = await getToken();
            if (!token) throw new Error("Not authenticated");
            return queryFn(token);
        },
        ...options,
    });
}

// Usage:
export function useProjects(offset = 0, limit = 20) {
    return useAuthedQuery(
        ["projects", offset, limit],
        (token) => api.projects.list(token, offset, limit),
    );
}
```

**Effort:** 1 day

---

### P2-4: No TypeScript Type Generation from Rust

**Impact:** Types are manually mirrored between Rust DTOs and TypeScript interfaces. Will drift as the API evolves.

**Current:** `apps/web/src/lib/api-client.ts` has 13 hand-written TypeScript interfaces matching Rust DTOs. If Rust adds a field, TypeScript won't know until runtime.

**Fix:** Use `typeshare` (generates TypeScript from Rust types) or `utoipa` (generates OpenAPI spec, then use `openapi-typescript` to generate TS).

**Effort:** 1-2 days

---

### P2-5: Hardcoded Evaluation Benchmarks

**Impact:** Benchmarks are embedded JSON files (`general_benchmark.json`, `safety_prompts.json`). Cannot customize per-tenant or per-project. Cannot version benchmark sets.

**Fix:**
```python
class BenchmarkSource(Protocol):
    async def load(self, benchmark_id: str) -> list[dict]: ...

class FileBenchmarkSource(BenchmarkSource):     # Current behavior
    ...
class S3BenchmarkSource(BenchmarkSource):       # Per-tenant benchmarks
    ...
class DatabaseBenchmarkSource(BenchmarkSource): # User-uploaded benchmarks
    ...
```

**Effort:** 1-2 days

---

### P2-6: Iterative Training Belongs in Workflow Layer

**Impact:** Iterative training (train → evaluate → retrain N times) is currently a special mode inside the `start_training` activity. This violates Temporal's architecture: workflows orchestrate, activities compute.

**Current:** `_train_iterative()` in `train_model.py` runs a loop of SFT rounds inside a single activity. If the activity crashes at iteration 4 of 5, all progress is lost — Temporal can only retry the entire activity.

**Fix:** Move to a dedicated workflow:
```python
@workflow.defn
class IterativeTrainWorkflow:
    @workflow.run
    async def run(self, input: StartTrainingInput):
        for i in range(input.hyperparams.get("num_iterations", 3)):
            # Each iteration is a separate activity (survives crashes)
            result = await workflow.execute_activity(start_training, ...)
            eval_result = await workflow.execute_activity(run_evaluation, ...)
```

**Effort:** 2 days

---

### P2-7: LLM Judge Logic Scattered Across Files

**Impact:** Three separate functions call the LLM judge with embedded heuristic fallbacks in `train_model.py`, plus the `JudgeLLM` class in `run_evaluation.py`. Different fallback logic in each. No shared abstraction.

| Location | Function | Fallback |
|---|---|---|
| `train_model.py:749` | `_score_response()` | Length heuristic |
| `train_model.py:805` | `_llm_reward_score()` | Keyword matching |
| `run_evaluation.py:429` | `JudgeLLM` class | Magic number scores (3/5) |

**Fix:** Unified `LLMJudge` protocol used everywhere:
```python
class LLMJudge(Protocol):
    async def score_response(self, prompt: str, response: str) -> float: ...
    async def compare_ab(self, prompt: str, response_a: str, response_b: str) -> str: ...
    def heuristic_fallback(self, response: str) -> float: ...
```

**Effort:** 1 day

---

### P2-8: No Request Timeout/Retry in Frontend API Client

**Impact:** `fetch()` defaults to no timeout. Slow backend endpoints will hang the UI forever. No retry logic for transient failures.

```typescript
// CURRENT: No timeout, no retry
const res = await fetch(`${API_URL}${path}`, { ...fetchOptions, headers });
```

**Fix:**
```typescript
const controller = new AbortController();
const timeoutId = setTimeout(() => controller.abort(), 30000);
const res = await fetch(`${API_URL}${path}`, {
    ...fetchOptions, headers,
    signal: controller.signal,
});
clearTimeout(timeoutId);
```

**Effort:** 0.5 day

---

### P2-9: Fire-and-Forget Tasks Swallow Errors

**Impact:** `tokio::spawn` in `api_key_service.rs` and `inference.rs` for background updates and billing events silently swallows errors. No visibility into whether these completed.

```rust
// CURRENT: Error silently dropped
tokio::spawn(async move {
    let _ = ApiKeyRepo::update_last_used(&db_clone, key_id).await;
});
```

**Fix:**
```rust
tokio::spawn(async move {
    if let Err(e) = ApiKeyRepo::update_last_used(&db_clone, key_id).await {
        tracing::warn!(key_id = %key_id, error = %e, "Failed to update last_used_at");
    }
});
```

**Effort:** 0.5 day

---

### P2-10: Metrics/Config Stored as Untyped JSON

**Impact:** `training_jobs.metrics`, `projects.config`, `documents.metadata`, `datasets.stats` are all `serde_json::Value` — untyped JSON blobs with no schema validation.

```rust
pub struct TrainingJob {
    pub metrics: serde_json::Value,      // Could be anything
    pub hyperparams: serde_json::Value,  // Could be anything
}
```

**Fix:** Define typed structs:
```rust
#[derive(Serialize, Deserialize)]
pub struct TrainingMetrics {
    pub loss: f64,
    pub learning_rate: f64,
    pub epoch: f64,
}

#[derive(Serialize, Deserialize)]
pub struct Hyperparams {
    pub r: i32,
    pub lora_alpha: i32,
    pub learning_rate: f64,
    // ...
}
```

**Effort:** 1-2 days

---

## Low-Priority Issues (P3)

Technical debt to track. Address opportunistically.

| ID | Issue | Layer | Impact | Effort |
|---|---|---|---|---|
| P3-1 | **No Sentry/error tracking** — Frontend errors are invisible in production | Frontend | Users experience silent failures | 0.5 day |
| P3-2 | **No form validation library** — No zod/react-hook-form. User input validated only server-side | Frontend | Poor UX, unnecessary round-trips | 1-2 days |
| P3-3 | **No pre-commit hooks** — Developers can commit unlinted/unformatted code | Infra | CI failures after commit | 0.5 day |
| P3-4 | **Billing events not partitioned** — `billing_events` table will grow to billions of rows at scale | DB | Slow billing queries | 1 day |
| P3-5 | **No structured error context** — Cannot attach `tenant_id`, `operation`, `resource_type` to errors for observability | Rust | Harder debugging in production | 1 day |
| P3-6 | **Auth extractors not pluggable** — Adding OAuth2/SAML/OpenID requires modifying extractor code, not extending it | Rust | Auth vendor lock-in | 2 days |
| P3-7 | **Query key factories missing** — Cache invalidation keys are string literals scattered across hooks. Fragile. | Frontend | Silent cache bugs | 0.5 day |
| P3-8 | **No configuration validation at startup** — API starts even if Redis/S3 is down. Only discovered at first request | Rust | Silent startup failures | 0.5 day |
| P3-9 | **No document parser registry** — Adding PPTX/RTF requires modifying if-elif chain in `parse_document.py` | Python | Not extensible for new formats | 1 day |
| P3-10 | **No analytics/telemetry** — Can't measure feature usage, conversion funnel, or identify dead code | Frontend | No product insight | 1 day |
| P3-11 | **Polling instead of WebSockets** — 3-5s polling for real-time updates. At 10K users = 2K-3K polls/sec on server | Both | Server load at scale | 3 days |
| P3-12 | **No integration tests** — No tests for service → repo → DB flow. Only unit tests for error handling and key hashing | Rust | Limited confidence in refactoring | 3-5 days |
| P3-13 | **No Python activity tests** — Zero tests for training, evaluation, parsing activities | Python | Cannot verify ML pipeline correctness | 3-5 days |
| P3-14 | **Docker compose has no resource limits** — No CPU/memory caps for production | Infra | OOM crashes in prod | 0.5 day |
| P3-15 | **Hardcoded adapter naming** — `format!("adapter-{model_id}")` duplicated in deploy/undeploy | Rust | Naming drift risk | 0.5 day |

---

## SOLID Principles Assessment

### Rust API Layer

| Principle | Score | Assessment |
|---|---|---|
| **Single Responsibility** | 8/10 | Services, repos, routes all single-purpose. Minor: some services do too much orchestration. |
| **Open/Closed** | 5/10 | Adding new auth methods, error types, or workflow engines requires modifying existing code. No extension points. |
| **Liskov Substitution** | 8/10 | `ObjectStorage` trait is properly designed for substitution. Would be 10/10 if Temporal and Redis were trait-based too. |
| **Interface Segregation** | 7/10 | `ObjectStorage` trait is focused (5 methods). But AppState exposes everything to every route. |
| **Dependency Inversion** | 4/10 | Services depend on concrete `PgPool`, `TemporalClient`, `redis::ConnectionManager`. Should depend on abstractions. |

### Python Worker Layer

| Principle | Score | Assessment |
|---|---|---|
| **Single Responsibility** | 5/10 | `train_model.py` (882 lines) handles 4 training modes + callbacks + LLM judging + metrics + checkpoints. Should be split into strategy classes. |
| **Open/Closed** | 3/10 | Can't add new training modes, evaluation suites, or parsers without modifying existing code. No plugin architecture. |
| **Liskov Substitution** | 3/10 | No abstractions to substitute. Can't swap Unsloth, TRL, or Redis without rewriting modules. |
| **Interface Segregation** | 4/10 | `clients.py` is a god provider. Activities depend on everything. |
| **Dependency Inversion** | 2/10 | Activities depend on concrete `clients` module, `httpx.Client`, `boto3.client`. No protocols. No DI. |

### Frontend Layer

| Principle | Score | Assessment |
|---|---|---|
| **Single Responsibility** | 6/10 | Pages are too large (628 lines). Hooks are good. API client is well-organized. |
| **Open/Closed** | 7/10 | Adding new API endpoints is easy (append to `api` object). Hooks follow consistent pattern. |
| **Dependency Inversion** | 5/10 | Hooks depend on concrete `api` client. No abstraction for different API backends or mocking. |

---

## Extensibility Scorecard

How hard is it to perform common operations today, and how hard would it be with the recommended fixes?

| Operation | Today | With Fixes | Root Cause |
|---|---|---|---|
| **Add new training mode** (ORPO, KTO) | 4-6 hours, modify core file | 1 hour, new file only | No strategy pattern (P0-3) |
| **Swap ML library** (Unsloth → Axolotl) | 3-5 days, rewrite 300+ lines | 1-2 days, new engine class | No engine abstraction (P0-2) |
| **Add evaluation suite** (Bias Detection) | 3-4 hours, modify core file | 1 hour, new file only | No suite registry (P0-4) |
| **Swap workflow engine** (Temporal → Airflow) | 5+ days, touch every service | 2 days, new trait impl | No orchestrator trait (P0-1) |
| **Swap auth provider** (Clerk → Auth0) | 2-3 days, modify extractors | 1 day, new provider impl | Hardcoded auth (P3-6) |
| **Add document parser** (PPTX) | 2 hours, modify if-elif chain | 30 min, register parser | No parser registry (P3-9) |
| **Mock DB for unit tests** | Impossible (concrete PgPool) | Easy (trait-based repos) | No repo traits (P2-1) |
| **Custom benchmarks per tenant** | Impossible (hardcoded files) | Easy (S3/DB source) | Hardcoded benchmarks (P2-5) |
| **Add new API endpoint** | 30 min (new route + service + repo) | Same | Good pattern already |
| **Add new frontend page** | 1-2 hours (follow hook + page pattern) | Same | Good pattern already |

---

## Layer-by-Layer Scores

| Layer | Score | Strengths | Weaknesses |
|---|---|---|---|
| **Rust API** | 7.5/10 | Perfect multi-tenancy. Clean Route→Service→Repo. ObjectStorage trait. Parallel queries. Error handling. | Concrete Temporal/Redis. No repo traits. String status. Untyped JSON fields. |
| **Python Worker** | 6.0/10 | Thin workflows. Config management. S3 paths. Worker mode separation. Lazy ML imports. | Monolithic training activity. No abstractions. Global state. Tight ML lib coupling. No tests. |
| **Frontend** | 7.0/10 | Consistent hook patterns. Smart polling. Good TypeScript types. React Query usage. | Monolithic pages. Boilerplate duplication. No form validation. No error tracking. Manual type sync. |
| **Database** | 8.0/10 | Proper schema design. UUID PKs. Audit columns. Soft deletes. Multi-tenancy columns. | RLS not configured. Missing indexes. No partitioning strategy. |
| **Cross-cutting** | 7.0/10 | Shared enums. S3 path consistency. Clean Cargo workspace. Docker dev setup. | No type generation. No pre-commit hooks. No integration tests. |
| **Overall** | **7.2/10** | | |

---

## Recommended Refactoring Order

Prioritized by impact-to-effort ratio. P0 items should be addressed before Phase 4.

| Priority | Refactoring | Effort | Payoff | Blocks |
|---|---|---|---|---|
| **P0** | Training Strategy pattern | 2-3 days | Adding new modes becomes trivial. OCP compliance. | New training modes |
| **P0** | TrainingEngine protocol | 2-3 days | ML library swaps become painless. Future-proofing. | Library evolution |
| **P0** | WorkflowOrchestrator trait | 1-2 days | Testability + engine swaps. DIP compliance. | Service testing |
| **P0** | Evaluation Suite registry | 1-2 days | Adding suites becomes plug-and-play. OCP compliance. | New eval suites |
| **P1** | Status enums in DB models | 1 day | Compile-time safety. Eliminate string comparison bugs. | Nothing (easy win) |
| **P1** | RLS policies in PostgreSQL | 0.5 day | Defense-in-depth for multi-tenancy. | Nothing (security) |
| **P1** | Missing DB indexes | 0.5 day | Performance at scale (10K+ tenants). | Nothing (easy win) |
| **P1** | AppState trait abstraction | 1-2 days | DI for services. Testability. | Service testing |
| **P1** | Protocol-based DI for Python | 2-3 days | Testable activities. Swappable backends. | Activity testing |
| **P2** | TypeScript type generation | 1-2 days | No more manual type sync. Prevents drift. | API changes |
| **P2** | Extract frontend components | 1-2 days | Maintainable UI. Reusable components. | UI growth |
| **P2** | Hook factory | 1 day | DRY frontend code. | Nothing |
| **P2** | Unified LLM Judge | 1 day | Consistent judge across train + eval. | Nothing |
| **P2** | API client timeouts/retry | 0.5 day | Resilient frontend. | Nothing |
| **P2** | Fix fire-and-forget logging | 0.5 day | Visible background errors. | Nothing |

**Total P0 effort:** ~7-10 days
**Total P0+P1 effort:** ~12-16 days
**Total all priorities:** ~22-28 days

---

## Final Verdict

### Is this code production-ready?

**For an MVP with <100 users — yes.** The architecture is sound, patterns are consistent, multi-tenancy is properly enforced, and the full pipeline works end-to-end.

### Is this good quality extensible code?

**Not yet.** The missing abstractions (training engine, strategy pattern, orchestrator trait, suite registry) mean that every time the ML landscape shifts — and it shifts fast — you're modifying core files instead of adding new ones. That's the difference between a codebase that scales with the team and one that becomes a bottleneck.

### What's the risk of not fixing?

| Scenario | Without Fixes | With Fixes |
|---|---|---|
| New training framework released | 3-5 days to integrate, risk breaking existing modes | 1-2 days, existing modes untouched |
| Customer requests custom eval suite | 3-4 hours, modify shared code, deploy everything | 1 hour, new file, deploy independently |
| Temporal pricing increases, need to swap | 5+ days, touch every service file | 2 days, implement new trait |
| Security audit asks about RLS | "It's enabled but... no policies" | "Full tenant isolation at DB + app level" |
| Team grows from 2 to 10 engineers | Everyone modifies same files, merge conflicts | Engineers work on independent strategy/engine files |

### The good news

The fixes are surgical. You're not rebuilding anything — you're extracting protocols/traits from code that already works, then plugging existing implementations into those abstractions. The patterns are there in spirit; they just need to be formalized into proper interfaces.
