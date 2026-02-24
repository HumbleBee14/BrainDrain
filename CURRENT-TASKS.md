# Current Tasks — BrainDrain Platform Improvements

> This file tracks all planned improvements, ordered from easiest to most complex.
> Each task includes what was done, why, and technical details once completed.

---

## Phase B: Backend Light (Small Backend Changes + Frontend)

All Phase B tasks completed.

---

## Phase C: Core Engineering (Major Features)

### C2. Notification Delivery Worker
- **Status**: Done
- **Why**: Preferences/delivery tracking exist but nothing actually sends emails/webhooks
- **What was done**: Built poll-based background worker following the BillingBatcher lifecycle pattern:
  - Added `list_pending_deliveries(max_attempts, limit)` to `NotificationRepository` trait + `PgNotificationRepo` — fetches pending and failed-retryable deliveries across all tenants
  - Created `DeliveryWorker` service with `Mutex<Option<ShutdownHandle>>` pattern for idempotent graceful shutdown
  - Background `tokio::spawn` poll loop with configurable interval (10s default)
  - Processes pending deliveries: fetches preference → validates channel → dispatches webhook
  - SSRF protection on every dispatch (DNS can change between attempts)
  - Exponential backoff timeout: 10s base, capped at 30s (10, 20, 30, 30, 30)
  - Max 5 delivery attempts before permanent failure
  - Batch size of 50 deliveries per poll cycle
  - Email channel stubbed with structured logging (ready for email provider trait)
  - Graceful shutdown: processes one final batch before exiting, integrated via `tokio::join!` with billing batcher
  - 3 unit tests: private IP detection, public IP allowance, exponential backoff cap verification
- **Files**: `delivery_worker.rs` (new), `traits.rs`, `notification_repo.rs`, `app_state.rs`, `main.rs`, `services/mod.rs`

### C3. Training Job Comparison UI
- **Status**: Done
- **Why**: All metrics stored but no side-by-side comparison for iteration
- **What was done**: Built side-by-side training job comparison page with selection UI:
  - Created `/projects/[id]/compare` page accepting `?jobs=id1,id2` query params
  - Side-by-side layout with `ComparisonRow` component supporting `lower`/`higher` highlight (green for better values)
  - Job headers with color-coded cards (violet/emerald), status badges
  - Final training loss visual overlay bar chart
  - Configuration section: base model, method, mode, GPU class, status
  - Hyperparameters section: learning rate, epochs, batch size, LoRA rank/alpha, max seq length, warmup steps, gradient accumulation, optimizer, LR scheduler
  - Cost & Timing section: cost estimate, actual cost, duration, start time (with green highlights for lower cost/duration)
  - Training Results section: final loss, total steps, runtime, samples/sec (with appropriate lower/higher highlights)
  - Added checkbox selection on training job rows in project detail page (visible when 2+ jobs exist)
  - Compare button appears when 2 jobs selected, navigates to comparison page
  - Breadcrumb navigation back to project
- **Files**: `apps/web/src/app/(dashboard)/projects/[id]/compare/page.tsx`, `apps/web/src/app/(dashboard)/projects/[id]/page.tsx`

### C4. Audit Log Viewer Page
- **Status**: Done
- **Why**: Backend stores full audit events; only last 10 shown on dashboard
- **What was done**: Built dedicated audit log viewer with full filtering capabilities:
  - Added `action` filter parameter to `AuditLogFilterParams` DTO
  - Added `list_filtered()` and `count_filtered()` to `AuditLogRepository` trait + `PgAuditLogRepo` with composable SQL WHERE clauses using `$N::text IS NULL OR column = $N` pattern
  - Updated `GET /audit-logs` route to route between exact resource, filtered, and unfiltered query paths
  - Added `auditLogs.list()` API client method with dynamic query string builder
  - Added `useAuditLogs` hook with reactive query key params
  - Built `/settings/audit-log` page with: text search (client-side across action/resource/actor), resource type dropdown filter, action dropdown filter (populated from current data), paginated table (25 per page), CSV export, relative timestamps with full-time tooltips, color-coded action badges (create=green, delete=red, update=blue, reject=amber, notification=violet), metadata preview column, clear filters button
  - Added "Audit Log" tab to settings layout
- **Files**: `audit_log.rs` (DTO), `audit_log_repo.rs`, `traits.rs`, `audit_logs.rs` (route), `api-client.ts`, `use-audit-logs.ts`, `settings/audit-log/page.tsx`, `settings/layout.tsx`

### C5. Data Lineage / Provenance Graph
- **Status**: Done
- **Why**: No visual showing documents → datasets → jobs → models → deployments
- **What was done**: Built data lineage visualization page tracing provenance through FK relationships:
  - Created `/projects/[id]/lineage` page using existing hooks (no new API endpoints needed)
  - Traces entity relationships: `TrainingJob.dataset_id` → Dataset, `Model.training_job_id` → TrainingJob
  - Visual flow: Documents → Datasets → Training Jobs → Models with connectors between stages
  - Datasets show inline linked training jobs via horizontal connectors
  - Models show provenance trail (training job base model + source dataset name)
  - Orphan training jobs (deleted dataset) shown in separate dashed section
  - Color-coded status dots and badges per entity (pending=gray, in-progress=blue, review=amber, completed=green, failed=red)
  - All entities link to their detail pages
  - Status legend at bottom
  - "Data Lineage" entry button added to pipeline status header on project detail page
  - Breadcrumb navigation
- **Files**: `apps/web/src/app/(dashboard)/projects/[id]/lineage/page.tsx`, project detail page

### C6. Model A/B Playground
- **Status**: Done
- **Why**: Playground only chats with one model; side-by-side comparison is powerful
- **What was done**: Built split-screen A/B playground at project level:
  - Created `/projects/[id]/playground` page with dual-panel chat interface
  - Each panel has independent model selector filtered to deployed models (prevents selecting the same model twice)
  - Shared input sends the same message to both models simultaneously via `Promise.allSettled`
  - SSE streaming handled independently per panel with separate scroll refs
  - Auto-creates API keys per model on first use (`playground-ab` named keys)
  - Shared settings panel: system prompt, temperature slider, max tokens slider
  - `ChatPanel` component encapsulates per-model message display, loading state, error display
  - Color-coded panels (violet for Model A, emerald for Model B)
  - Clear All button resets both conversations
  - "A/B Playground" entry button on project detail page (visible when at least 1 deployed model)
  - Breadcrumb navigation
- **Files**: `apps/web/src/app/(dashboard)/projects/[id]/playground/page.tsx`, project detail page

### C7. Model Versioning & Rollback
- **Status**: Done
- **Why**: `version` field exists (always 1); no version history or rollback
- **What was done**: Implemented full model versioning with auto-increment and rollback:
  - Added `list_versions()` and `get_max_version()` to `ModelRepository` trait + `PgModelRepo` — queries models by `base_model` within a project, ordered by version DESC
  - Added `list_versions()` to `ModelService` — fetches model by ID, then lists all versions sharing the same `base_model`
  - Added `rollback()` to `ModelService` — validates same project/base_model, undeploys current if active, deploys target version
  - Added `GET /models/{id}/versions` route returning all versions as `Vec<ModelResponse>`
  - Added `POST /models/{id}/rollback` route (Admin role, audit logged) with `RollbackModelRequest` DTO
  - Auto-increment version in Python training worker: queries `MAX(version)` for the same `base_model` in the project before INSERT, sets `version = max + 1`
  - Added `listVersions()` and `rollback()` API client methods
  - Added `useModelVersions` and `useRollbackModel` React hooks with query invalidation
  - Added Version History section on model detail page: shows all versions with deployment status badges, "current" indicator, eval scores, rollback button per non-current version
  - Version History section only visible when more than 1 version exists
- **Files**: `traits.rs`, `model_repo.rs`, `model_service.rs`, `training.rs` (routes), `model.rs` (DTO), `train_model.py`, `api-client.ts`, `use-models.ts`, model detail page

### C8. Batch Inference Endpoint
- **Status**: Done
- **Why**: Only single-message completions; no bulk processing
- **What was done**: Added synchronous batch inference endpoint with concurrent processing:
  - `POST /v1/chat/completions/batch` — accepts up to 50 requests per batch
  - `BatchChatCompletionRequest` with array of `BatchRequestItem` (custom_id, messages, temperature, max_tokens, top_p)
  - Bounded concurrency: 5 simultaneous vLLM requests via `futures::stream::buffer_unordered`
  - Per-item error isolation: failed items return error string, successful items return full response
  - Aggregated billing: single `BillingEvent` with total prompt/completion tokens for the batch
  - `BatchChatCompletionResponse` with results array and `BatchUsageSummary` (totals, success/fail counts)
  - Same API key auth, model deployment validation, circuit breaker, and max_tokens cap as single endpoint
  - Batch metadata included in billing event for analytics
- **Files**: `inference.rs` (routes)

### C9. Admin Config Panel
- **Status**: Pending
- **Why**: GPU rates, model list, chunk sizes all hardcoded
- **Scope**: Tenant admin settings page exposing configuration
- **Files**: Backend settings API + frontend page

### C10. Iterative Training Early Stopping
- **Status**: Pending
- **Why**: Workflow exists but early stopping is a stub
- **Scope**: Implement validation metric computation between rounds
- **Files**: Python worker activities

---

## Completed Tasks

### C1. Streaming Inference (SSE)
- **Status**: Done (was already implemented)
- **Why**: Discovered during code audit that full SSE streaming already exists in `inference.rs`. The route reads `stream` param, sets `text/event-stream` headers, forwards vLLM SSE chunks via `Body::from_stream()`, captures token usage from final chunk for billing, and spawns async billing with conservative fallback on early disconnect. The playground frontend already uses `stream: true` with proper SSE parsing.

### B4. Webhook Testing & Retry
- **Status**: Done
- **Why**: Users could configure webhook URLs but had no way to test connectivity or retry failed deliveries
- **What was done**: Implemented webhook test and delivery retry capability:
  - Added `get_delivery` and `get_preference` to `NotificationRepository` trait + `PgNotificationRepo` implementation
  - Added `test_webhook()` to `NotificationService` — sends a test payload to the configured webhook URL, creates delivery record, returns result with status
  - Added `retry_delivery()` to `NotificationService` — re-sends original payload for failed deliveries, increments attempt counter
  - Added `POST /notifications/preferences/:id/test` route (Admin role, audit logged)
  - Added `POST /notifications/deliveries/:id/retry` route (Admin role, audit logged)
  - Added `testWebhook()` and `retryDelivery()` API client methods
  - Added `useTestWebhook` and `useRetryDelivery` React hooks with query invalidation
  - Added "Test Webhook" button next to webhook URL input (visible when saved, no unsaved changes)
  - Added "Retry" button per failed delivery row in delivery history table
  - Added inline error display for failed deliveries (truncated error message with tooltip)
  - SSRF protection applied to both test and retry paths via existing `is_safe_webhook_url()` validation
- **Files**: `traits.rs`, `notification_repo.rs`, `notification_service.rs`, `notifications.rs` (routes), `api-client.ts`, `use-notifications.ts`, notifications settings page

### A1. Toast Notifications for Mutations
- **Status**: Done
- **Why**: Users had no visual feedback when actions succeeded/failed
- **What was done**: Installed `sonner` toast library. Added `<Toaster>` to root layout with dark theme matching zinc color scheme. Wired toast notifications (success + error) into all mutation hooks across 8 pages:
  - Project page: upload, parse, refine, create training, cancel training, delete project
  - Dataset page: approve, reject
  - Model page: deploy, undeploy, revoke API key, create export
  - Evaluation page: create evaluation
  - Team settings: invite, role update, remove member, revoke invitation
  - LLM settings: save, reset
  - Notifications settings: save preferences
  - New project: create project
- **Files**: `apps/web/package.json`, `apps/web/src/app/layout.tsx`, + 8 page files

### A2. Breadcrumb Navigation
- **Status**: Done
- **Why**: Deep pages had only manual "Back" links with no navigation context
- **What was done**: Created reusable `<Breadcrumbs>` component at `src/components/breadcrumbs.tsx`. Shows hierarchical path with clickable links (`Projects / Project / Model / Evaluation`). Replaced all `← Back to X` links across 7 pages:
  - Project detail, dataset review, training job detail, model detail, evaluation, playground (2 states), new project
- **Files**: `apps/web/src/components/breadcrumbs.tsx`, + 7 page files

### A3. Search & Filter on Lists
- **Status**: Done
- **Why**: Documents and training jobs were flat lists with no way to find specific items
- **What was done**: Added client-side filtering to the project detail page:
  - **Documents**: Search by filename + filter by status (uploaded/parsing/parsed/failed). Shows "X of Y" count when filtered. Only visible when >3 documents.
  - **Training Jobs**: Filter by status (pending/training/completed/failed/cancelled). Shows "X of Y" count when filtered. Only visible when >3 jobs. Empty state message adjusts for active filters.
- **Files**: `apps/web/src/app/(dashboard)/projects/[id]/page.tsx`

### A4. Hyperparameter Presets
- **Status**: Done
- **Why**: Users had to manually configure every training run; presets reduce decision fatigue
- **What was done**: Added 4 one-click presets above the training form:
  - **Quick Experiment**: Llama 3.2 1B, QLoRA, Quick mode — fastest iteration
  - **Balanced**: Llama 3.2 3B, QLoRA, Aligned (SFT+DPO) — good quality/speed tradeoff
  - **Production**: Llama 3.1 8B, LoRA, Aligned, A10G GPU — production-grade output
  - **Max Quality**: Llama 3.1 8B, LoRA, Reasoning (GRPO), L40S GPU — highest quality
  Each preset populates method, mode, base_model, and gpu_class in the form. Tooltip shows description.
- **Files**: `apps/web/src/app/(dashboard)/projects/[id]/page.tsx`

### A5. Cost Variance Display (Estimated vs Actual)
- **Status**: Done
- **Why**: `actual_cost` is populated post-training but users couldn't compare it to the estimate
- **What was done**: Enhanced the training job detail "Timing & Cost" card:
  - Added **Duration** row (computed from started_at/completed_at, shows Xh Ym format)
  - Added **Cost Variance** row when both estimated and actual cost exist. Shows dollar difference and percentage. Color-coded: amber for over-budget, emerald for under-budget. Format: `+$0.15 (+12%)` or `-$0.30 (-25%)`
- **Files**: `apps/web/src/app/(dashboard)/projects/[id]/training/[jobId]/page.tsx`

### A6. Training Progress Indicators
- **Status**: Done
- **Why**: Active training showed no progress indication beyond a spinner
- **What was done**: Added an animated progress bar on the training job detail page, visible during active training when metrics are streaming. Shows:
  - Current epoch vs total epochs (e.g., `Epoch 1.54 / 3`)
  - Percentage complete with smooth purple bar (`51%`)
  - Total epochs derived from `hyperparams.num_train_epochs` or `hyperparams.epochs`, defaulting to 3
  - Smooth 500ms CSS transition for bar width updates
- **Files**: `apps/web/src/app/(dashboard)/projects/[id]/training/[jobId]/page.tsx`

### B1. Full Pipeline One-Click Endpoint
- **Status**: Done
- **Why**: `FullPipelineWorkflow` exists in Python workers but had no API endpoint; users had to manually trigger each pipeline stage (parse → refine → train → evaluate) individually
- **What was done**: Added end-to-end "one-click fine-tune" capability:
  - Added `start_full_pipeline()` method to `WorkflowOrchestrator` trait and `TemporalClient` implementation — starts `FullPipelineWorkflow` with all parameters (tenant_id, project_id, document_ids, task_type, base_model, training_config)
  - Added `trigger_full_pipeline()` to `PipelineService` — collects all uploaded + parsed documents and starts the workflow
  - Added `POST /projects/{project_id}/full-pipeline` route with audit logging
  - Added `TriggerFullPipelineRequest` and `TriggerFullPipelineResponse` DTOs with ts-rs export
  - Added `triggerFullPipeline()` API client method and `useTriggerFullPipeline` React hook
  - Added violet "One-Click Fine-Tune" button in project detail page pipeline actions, with toast notifications
- **Files**: `temporal.rs`, `pipeline_service.rs`, `pipeline.rs` (routes), `dto/pipeline.rs`, `api-client.ts`, `use-pipeline.ts`, project detail page

### B2. Rate Limit Enforcement Middleware
- **Status**: Done (was already implemented)
- **Why**: Discovered during code audit that rate limiting already exists in `ApiKeyService::authenticate()` using a Redis Lua script per-minute sliding window. No additional work needed.

### B3. Cost Approval Workflow
- **Status**: Done
- **Why**: `TrainingJobStatus::CostApproval` enum existed but was never used; prevents surprise training bills by requiring manual approval for expensive jobs
- **What was done**: Implemented complete cost approval workflow:
  - Added configurable cost threshold (default $5.00) — jobs above this cost are paused before starting
  - Modified `TrainingJobService::create()` to check `cost_estimate > threshold`; if exceeded, transitions job to `cost_approval` status and does NOT start the Temporal workflow
  - Added `set_cost_approval()` and `approve_cost()` methods to `TrainingJobRepository` trait + `PgTrainingJobRepo` implementation
  - Added `approve_cost()` to `TrainingJobService` — transitions from `cost_approval → pending`, then starts the TrainWorkflow
  - Added `POST /training-jobs/{id}/approve-cost` route (requires Admin role) with audit logging
  - Cancel already handled `cost_approval` status (existing `cancel()` method)
  - Added `approveCost()` API client method and `useApproveCost` React hook
  - Added amber cost approval banner on training job detail page with Approve/Reject buttons
- **Files**: `training_job_service.rs`, `training.rs` (routes), `traits.rs`, `training_job_repo.rs`, `api-client.ts`, `use-training.ts`, training job detail page

### B5. Project Status State Machine
- **Status**: Done
- **Why**: 7 project statuses (Created, Ingesting, Refining, Training, Evaluating, Deployed, Archived) existed but no transition validation; projects could jump to any status randomly
- **What was done**: Added state machine validation with allowed transitions:
  - Forward pipeline: Created → Ingesting → Refining → Training → Evaluating → Deployed
  - Rollback on failure: each step can go back one step
  - Archive: Created/Deployed → Archived, Archived → Created (un-archive)
  - Re-evaluate: Deployed → Evaluating
  - Added `is_valid_transition()` function with pattern matching
  - Added `update_status()` to `ProjectService` with validation
  - Added `update_status()` to `ProjectRepository` trait + `PgProjectRepo` implementation
  - Added `PUT /projects/{id}/status` route with `UpdateProjectStatusRequest` DTO
  - Added 5 unit tests covering forward, rollback, archive, invalid, and same-status transitions
- **Files**: `project_service.rs`, `projects.rs` (routes), `project.rs` (DTO), `traits.rs`, `project_repo.rs`

---

*Last updated: 2026-02-24*
