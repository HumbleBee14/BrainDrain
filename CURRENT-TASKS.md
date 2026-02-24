# Current Tasks — BrainDrain Platform Improvements

> This file tracks all planned improvements, ordered from easiest to most complex.
> Each task includes what was done, why, and technical details once completed.

---

## Phase B: Backend Light (Small Backend Changes + Frontend)

### B4. Webhook Testing & Retry
- **Status**: Pending
- **Why**: Users can configure webhook URLs but can't test them or retry failed deliveries
- **Scope**: Add test endpoint + retry button in notification UI
- **Files**: Backend route + frontend

---

## Phase C: Core Engineering (Major Features)

### C1. Streaming Inference (SSE)
- **Status**: Pending
- **Why**: Inference endpoint accepts `stream` param but ignores it; standard for LLM APIs
- **Scope**: Implement SSE streaming in inference route, proxy to vLLM streaming
- **Files**: Backend route + vLLM client

### C2. Notification Delivery Worker
- **Status**: Pending
- **Why**: Preferences/delivery tracking exist but nothing actually sends emails/webhooks
- **Scope**: Add background worker that processes `notification_deliveries` table
- **Files**: Backend worker/service

### C3. Training Job Comparison UI
- **Status**: Pending
- **Why**: All metrics stored but no side-by-side comparison for iteration
- **Scope**: New comparison page with metrics overlay
- **Files**: Frontend page + possibly new API endpoint

### C4. Audit Log Viewer Page
- **Status**: Pending
- **Why**: Backend stores full audit events; only last 10 shown on dashboard
- **Scope**: New dedicated page with search, filter, pagination, export
- **Files**: Frontend page (backend route already exists)

### C5. Data Lineage / Provenance Graph
- **Status**: Pending
- **Why**: No visual showing documents → datasets → jobs → models → deployments
- **Scope**: New visualization component tracing data provenance
- **Files**: Frontend page + possibly new API endpoint

### C6. Model A/B Playground
- **Status**: Pending
- **Why**: Playground only chats with one model; side-by-side comparison is powerful
- **Scope**: Split-screen playground with two model selections
- **Files**: Frontend page

### C7. Model Versioning & Rollback
- **Status**: Pending
- **Why**: `version` field exists (always 1); no version history or rollback
- **Scope**: Version incrementing logic, version history API, rollback UI
- **Files**: Backend service + routes + frontend

### C8. Batch Inference Endpoint
- **Status**: Pending
- **Why**: Only single-message completions; no bulk processing
- **Scope**: New batch endpoint with job queue
- **Files**: Backend route + service + worker

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
