# BrainDrain — Platform Improvement Roadmap

This document tracks all critical gaps and planned improvements to make BrainDrain
the definitive LLM fine-tuning and inference platform: configurable with sensible
defaults, usable by anyone, and production-grade throughout.

Each item is marked with its implementation status.

---

## Priority 1 — Critical (Platform Correctness)

### 1. Real Evaluation Suites ✅
**Problem**: Evaluation workflow runs but all suites are stubs — no actual benchmarks,
no perplexity/BLEU/ROUGE, no real LLM-as-judge scoring, no A/B comparison.
A training platform without real evals is a black box.

**Implementation**:
- `DomainEvaluation`: LLM-as-Judge scoring on held-out val pairs (configurable judge model)
- `GeneralCapability`: Perplexity on held-out text, ROUGE-L vs reference answers
- `ABComparison`: Blind pairwise comparison of fine-tuned vs base model
- `SafetyCheck`: Adversarial prompt battery with refusal rate tracking
- Aggregate weighted score stored in `eval_scores` JSONB

**Config** (all optional, sensible defaults):
- `APP_EVAL_JUDGE_MODEL` — model for LLM-as-judge (default: `gpt-4o-mini`)
- `APP_EVAL_JUDGE_TEMPERATURE` — judge temperature (default: `0.0`)
- `APP_EVAL_MAX_SAMPLES` — cap samples per suite (default: `50`)
- `APP_EVAL_SUITE_WEIGHTS` — JSON weight map per suite (default: equal)

---

### 2. WebSocket Real-Time Training Metrics ✅
**Problem**: `ws.rs` route exists but has no Redis pub/sub wired in. Dashboard polls
for metrics, causing stale data and poor UX during multi-hour training runs.

**Implementation**:
- Workers publish metrics events to Redis channel `training:{job_id}:metrics`
- API WebSocket handler subscribes and streams to client
- Client receives live loss curves, step count, ETA
- Graceful close on job completion or client disconnect

---

### 3. Hardcoded Limits → Config ✅
**Problem**: Critical tunables are hardcoded in source:
- `MAX_TOKENS_LIMIT: 8192` in inference route
- Workflow activity timeouts (10min parse, 30min refine, 6hr train)
- Chunk size and overlap in `chunk_text`
- Upload size limit (500MB)
- LLM retry/circuit-breaker thresholds

**Implementation**: All moved to `AppConfig` / `WorkerSettings` with env var overrides
and documented defaults in `.env.example`.

---

### 4. S3 Partial Failure Recovery ✅
**Problem**: A single document parse failure halts the entire ingestion pipeline.
One corrupted PDF ruins a batch of 50 docs.

**Implementation**:
- `IngestWorkflow` tracks per-document success/failure independently
- Failed documents are marked `status="failed"` with `error_message`
- Pipeline continues and reports partial success
- UI shows which documents failed and why
- Re-trigger only failed documents without re-processing successful ones

---

### 5. Complete GGUF Export with Quantization Options ✅
**Problem**: Export activity is registered but untested and supports only one
implicit quantization format.

**Implementation**:
- Support `Q4_K_M`, `Q5_K_M`, `Q8_0`, `F16` quantization types
- Quantization type configurable at export time (UI dropdown + API param)
- File size estimate shown before export starts
- Upload quantized GGUF to S3, update `model_exports` with file size + format

---

## Priority 2 — Security & Reliability

### 6. Per-API-Key Rate Limiting ✅
**Problem**: `api_keys.rate_limit` column exists but middleware never checks it.
Any key can send unlimited inference requests.

**Implementation**:
- Redis sliding window counter per API key (`ratelimit:{key_id}:{window}`)
- Limit read from DB at key validation time (cached in Redis 60s)
- Returns `429 Too Many Requests` with `Retry-After` header
- Per-key limit overrides global `RATE_LIMIT_RPM`

---

### 7. Row-Level Security (RLS) Enforcement ✅
**Problem**: Schema has `tenant_id` everywhere and RLS placeholder policies,
but they aren't active. One Rust bug could leak cross-tenant data.

**Implementation**:
- Enable RLS on all tenant-scoped tables
- `SET LOCAL app.tenant_id = $1` on every connection from a pool
- Policy: `USING (tenant_id = current_setting('app.tenant_id')::uuid)`
- Migration added, tested with cross-tenant query attempts

---

## Priority 3 — Inference & Scale

### 8. Streaming Inference (SSE) ✅
**Problem**: `/v1/chat/completions` returns only complete responses.
vLLM supports streaming natively — not piping it through means slow TTFT for users.

**Implementation**:
- Detect `"stream": true` in request body
- Proxy vLLM SSE stream directly to client with `Transfer-Encoding: chunked`
- Emit OpenAI-compatible `data: {"choices":[{"delta":...}]}` chunks
- Emit `data: [DONE]` on completion

---

### 9. Multi-Adapter Serving on One Base Model

**Problem**: Each deployment occupies a separate vLLM instance. Loading 10 fine-tuned
Llama-3-8B adapters = 10× GPU memory for base weights.

#### Phase 1 — Adapter Limit Enforcement (Done)

What we built: **guard rail on a single vLLM endpoint**.

- `DeploymentService::deploy()` counts active adapters sharing the same `base_model`
  before calling vLLM's `/v1/load_lora_adapter`
- If count >= `VLLM_MAX_LORAS` (configurable, default 4), returns 409 Conflict with
  a clear message instead of a cryptic vLLM error
- New `ModelRepository::count_active_by_base_model()` query
- This works correctly for the current single-vLLM-instance architecture

**What this is NOT**: a scheduler that manages multiple vLLM instances or routes
deploys to the right one. Multiple adapters sharing one base model's GPU memory
is already how vLLM works natively — we just prevent overloading it.

#### Phase 2 — Multi-Instance Scheduling (Future)

What would need to be built for true instance-aware placement:

- A `vllm_instances` table tracking: instance URL, base model loaded, current
  adapter count, GPU type, health status, last heartbeat
- Placement algorithm: "find healthy instance running Llama-3-8B with free
  adapter slots" → load adapter there; if none found → provision new instance
- Instance provisioning: spin up vLLM via Docker/K8s API when no compatible
  instance exists, tear down when all adapters unloaded (scale-to-zero)
- Health check loop: periodic probe of each instance, mark unhealthy on failure
- This is essentially a mini scheduler — significant scope, requires multi-node
  infrastructure (Kubernetes or equivalent) to be meaningful

**Why not now**: The current architecture runs a single vLLM instance (see
`docker-compose.yml`). Building a scheduler without multiple instances to
schedule against would be untestable code. Phase 1 (limit enforcement) is the
correct guard rail for single-instance deployments. Phase 2 becomes relevant
when the platform scales to multiple GPU nodes.

**Does deferring Phase 2 break anything?** No.
- Single model training is completely unaffected — training uses Unsloth/TRL
  directly on GPU, never touches vLLM
- Single-instance inference works correctly with Phase 1's limit enforcement
- Multiple adapters on one instance already share base model memory (vLLM native)
- The only limitation: when `VLLM_MAX_LORAS` is full, users must undeploy
  an existing model before deploying a new one (clear error message explains this)

---

## Priority 4 — UX & Non-Technical Users

### 10. Smart Defaults & Auto-Configuration
**Problem**: Users must manually pick base model, training mode, chunk size, etc.
with no guidance. Non-technical users are immediately overwhelmed.

**Implementation**:
- Task-type → base model suggestions (QA→Llama-3-8B, code→Qwen2.5-Coder, etc.)
- Dataset size → training mode auto-select (< 1K pairs → `quick`, > 5K → `aligned`)
- Cost + time estimate displayed before job submission (based on pair count × GPU type)
- Config field tooltips explaining each parameter in plain language

---

### 11. Progress + ETA on Long Training Jobs
**Problem**: Training jobs show a spinner with no time estimate. 6-hour GPU runs with
no ETA cause users to cancel perfectly healthy jobs.

**Implementation**:
- Compute `eta_seconds = (total_steps - current_step) / steps_per_second`
- Emit via Redis metrics channel alongside loss
- Display as "~2h 14m remaining" in dashboard
- Show step progress bar (step N of M)

---

### 12. Guided Onboarding Flow
**Problem**: New users see an empty dashboard with no "start here" path.

**Implementation**:
- 3-step wizard: Upload → Configure → Launch
- Sample documents available for first-time users ("Try with our sample data")
- Completion checklist (parse ✓ → refine ✓ → train ✓ → evaluate ✓ → deploy ✓)
- Dismissible after first successful training job

---

## Priority 5 — Operational Excellence

### 13. Fix Team Member O(N) Query ✅
**Problem**: `team_service.rs` has a TODO to replace membership check with
`SELECT EXISTS` instead of fetching full records.

**Implementation**: One-line fix — `SELECT EXISTS(SELECT 1 FROM team_members WHERE ...)`.

---

### 14. Billing Credit-Back for Failed Jobs
**Problem**: Failed training jobs still log billing events. Users charged for GPU time
on jobs that crashed immediately.

**Implementation**:
- On job failure, check elapsed GPU time
- If < `MIN_BILLABLE_MINUTES` (configurable, default 5), void the billing event
- Emit credit event to Stripe for partially-used GPU time

---

### 15. In-Memory ObjectStorage Mock for Testing
**Problem**: Unit tests currently hit real MinIO. No `MockStorage` implementation
of the `ObjectStorage` trait makes testing slow and environment-dependent.

**Implementation**:
- `InMemoryStorage` struct with `HashMap<String, Bytes>` backing
- Implements full `ObjectStorage` trait
- Available via `cfg(test)` or `STORAGE_BACKEND=memory` env var
- Unblocks fast unit tests for all activities

---

## Implementation Order

| # | Feature | Est. Complexity | Status |
|---|---------|----------------|--------|
| 1 | Real evaluation suites | High | ✅ Done (already implemented) |
| 2 | WebSocket metrics streaming | Medium | ✅ Done — Redis XREAD wired |
| 3 | Hardcoded limits → config | Low | ✅ Done — timeouts.py + INFERENCE_MAX_TOKENS |
| 4 | S3 partial failure recovery | Medium | ✅ Done (already implemented) |
| 5 | GGUF export + quant options | Medium | ✅ Done (already implemented) |
| 6 | Per-API-key rate limiting | Medium | ✅ Done (already implemented) |
| 7 | RLS enforcement | Medium | ✅ Done — before_acquire + migration 009 |
| 8 | Streaming inference (SSE) | Medium | ✅ Done (already implemented) |
| 9 | Multi-adapter serving | High | ✅ Done — adapter limit enforcement + VLLM_MAX_LORAS |
| 10 | Smart defaults / auto-config | High | ✅ Done — GET /api/v1/models/catalog + auto-suggest |
| 11 | Training ETA display | Low | ✅ Done — eta_seconds in metrics stream |
| 12 | Onboarding wizard | High | ✅ Done (already implemented — onboarding-banner.tsx) |
| 13 | Team query O(1) fix | Trivial | ✅ Done — SELECT EXISTS |
| 14 | Billing credit-back | Medium | ✅ Done — _maybe_void_billing + MIN_BILLABLE_SECONDS |
| 15 | In-memory storage mock | Low | ✅ Done — InMemoryStorage + 10 tests |
| 16 | Safety benchmark coverage | Low | ✅ Done — 30 → 65 prompts, 5 categories |
