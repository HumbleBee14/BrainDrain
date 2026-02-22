# Phase 5: Serving & Deployment — Complete

Production-ready inference infrastructure. Circuit breaker for resilience, SSE streaming for real-time playground, GGUF export for local inference, and usage metering for billing visibility.

## What Was Built

### Task 1: Infrastructure Hardening

| Component | File | What It Does |
|---|---|---|
| Circuit breaker | `crates/api/src/services/circuit_breaker.rs` | Async state machine (Closed → Open → HalfOpen) protecting vLLM calls. Trips after 5 consecutive failures, recovers after 30s. Returns 503 `ServiceUnavailable` when open. |
| Billing micro-batcher | `crates/api/src/services/billing_batcher.rs` | Channel-based background worker. Collects billing events via `mpsc::Sender`, bulk-inserts every 5s or 1000 items via `sqlx::QueryBuilder`. Replaces per-request `tokio::spawn` DB writes. |
| Dashboard Redis cache | `crates/api/src/services/dashboard_service.rs` | 30s TTL for stats, 60s TTL for usage. Avoids 7 parallel queries on every dashboard load. |
| `ServiceUnavailable` error | `crates/api/src/error.rs` | New `AppError` variant mapping to HTTP 503. |

### Task 2: SSE Streaming Inference

| Component | File | What It Does |
|---|---|---|
| Streaming proxy | `crates/api/src/routes/inference.rs` | When `stream: true`, forwards vLLM's `text/event-stream` bytes directly via `Body::from_stream()`. Extracts usage from final SSE chunk for billing. |
| Frontend streaming | `apps/web/.../playground/page.tsx` | `ReadableStream` consumption with incremental token display. Parses SSE `data:` lines, appends `delta.content` to assistant message. |

### Task 3: Real Deploy Activity

| Component | File | What It Does |
|---|---|---|
| Deploy activity | `apps/workers/src/activities/stubs.py` | Replaced stub with real `aiohttp` call to Rust API's `/api/v1/models/{id}/deploy`. 300s timeout, error propagation. |

### Task 4: GGUF Export Pipeline

| Component | File | What It Does |
|---|---|---|
| DB migration | `crates/db/src/migrations/008_exports.sql` | `model_exports` table with RLS policy and index. |
| Model | `crates/db/src/models.rs` | `ModelExport` struct. |
| DTOs | `crates/api/src/dto/export.rs` | `ExportRequest`, `ExportResponse`, `ExportDownloadResponse` with ts-rs export. |
| Repository | `crates/api/src/repositories/export_repo.rs` | `PgExportRepo` — create, get, list, update_status. |
| Service | `crates/api/src/services/export_service.rs` | Validates quant type, creates DB row, triggers Temporal workflow, generates presigned download URLs. |
| Routes | `crates/api/src/routes/exports.rs` | `POST /models/{id}/exports`, `GET /models/{id}/exports`, `GET /exports/{id}/download`. |
| Temporal workflow | `apps/workers/src/workflows/export.py` | `ExportWorkflow` wrapping the GGUF activity. |
| GGUF activity | `apps/workers/src/activities/export_gguf.py` | Downloads adapter from S3, merges LoRA via peft, converts to GGUF via llama.cpp, quantizes (Q4_K_M/Q5_K_M/Q6_K/Q8_0), uploads to S3. |
| Frontend | `apps/web/.../models/[modelId]/page.tsx` | Export section with quant type dropdown, export button, status badges, download links. |
| Hook | `apps/web/src/hooks/use-exports.ts` | `useModelExports`, `useCreateExport`, `useExportDownload`. |

### Task 5: Usage Dashboard

| Component | File | What It Does |
|---|---|---|
| Inference usage query | `crates/api/src/repositories/billing_event_repo.rs` | `inference_usage_by_day` — daily breakdown of request count, prompt/completion tokens, cost. |
| Usage endpoint | `crates/api/src/routes/dashboard.rs` | `GET /dashboard/inference-usage` returning last 30 days of inference data. |
| Usage page | `apps/web/.../settings/usage/page.tsx` | Summary cards, bar charts (requests/day, tokens/day), daily breakdown table. |
| Settings nav | `apps/web/.../settings/layout.tsx` | Added "Usage" tab. |

## Files Created (13)

- `crates/api/src/services/circuit_breaker.rs`
- `crates/api/src/services/billing_batcher.rs`
- `crates/api/src/services/export_service.rs`
- `crates/api/src/routes/exports.rs`
- `crates/api/src/dto/export.rs`
- `crates/api/src/repositories/export_repo.rs`
- `crates/db/src/migrations/008_exports.sql`
- `apps/workers/src/activities/export_gguf.py`
- `apps/workers/src/workflows/export.py`
- `apps/web/src/hooks/use-exports.ts`
- `apps/web/src/app/(dashboard)/settings/usage/page.tsx`
- `docs/phase5/PHASE5_COMPLETE.md`

## Files Modified (20+)

- `crates/api/src/error.rs` — `ServiceUnavailable` variant
- `crates/api/src/app_state.rs` — circuit breaker, billing batcher, export repo
- `crates/api/src/routes/inference.rs` — circuit breaker + SSE streaming + billing batcher
- `crates/api/src/routes/deployments.rs` — shared HTTP client + circuit breaker
- `crates/api/src/routes/dashboard.rs` — inference usage endpoint
- `crates/api/src/routes/mod.rs` — export routes
- `crates/api/src/services/deployment_service.rs` — circuit breaker wrapping
- `crates/api/src/services/dashboard_service.rs` — Redis caching
- `crates/api/src/services/mod.rs` — new modules
- `crates/api/src/dto/mod.rs` — export DTO
- `crates/api/src/dto/dashboard.rs` — Deserialize for caching
- `crates/api/src/repositories/traits.rs` — ExportRepository trait + inference_usage_by_day
- `crates/api/src/repositories/mod.rs` — export_repo
- `crates/api/src/repositories/billing_event_repo.rs` — InferenceUsageDay + query
- `crates/api/src/temporal.rs` — start_export method
- `crates/db/src/models.rs` — ModelExport struct
- `Cargo.toml` — reqwest stream feature
- `apps/workers/src/worker.py` — ExportWorkflow + ExportGgufActivity
- `apps/workers/src/activities/stubs.py` — real deploy activity
- `apps/workers/src/config.py` — platform_api_url, vllm_api_url
- `apps/web/src/lib/api-client.ts` — exports + inference-usage API
- `apps/web/src/app/(dashboard)/projects/[id]/models/[modelId]/page.tsx` — export section
- `apps/web/src/app/(dashboard)/projects/[id]/models/[modelId]/playground/page.tsx` — SSE streaming
- `apps/web/src/app/(dashboard)/settings/layout.tsx` — Usage tab

## Verification

- `cargo clippy --workspace -- -D warnings` — 0 warnings
- `npx tsc --noEmit` — 0 TypeScript errors
- Circuit breaker has 5 unit tests (closed, trips, resets, recovers, half-open failure)
- Deployment service has 13 unit tests
