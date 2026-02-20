# Phase 1 — Data Pipeline (Complete)

> Upload a document, parse it, generate synthetic training data, review the dataset — all working end-to-end.

## What Was Built

Phase 1 delivers the complete data pipeline: document parsing (6 formats), text chunking, LLM-powered synthetic pair generation, dataset assembly with quality filtering, and a frontend for uploading, monitoring, and reviewing results. The Temporal workflow stubs from Phase 0 are now fully implemented with real ML activities. A Rust-side Temporal client enables the API to trigger workflows, and 7 new API endpoints expose pipeline operations to the frontend.

---

## New Files Added

```
BrainDrain/
├── docs/
│   └── phase1/
│       └── PHASE1_COMPLETE.md              # This file
│
├── crates/api/src/
│   ├── temporal.rs                         # HTTP-based Temporal client (no gRPC dep)
│   ├── routes/
│   │   ├── pipeline.rs                     # POST parse, POST refine, GET status
│   │   └── datasets.rs                     # GET datasets (list, get, preview, parsed content)
│   ├── services/
│   │   ├── pipeline_service.rs             # Parse/refine trigger logic, status aggregation
│   │   └── dataset_service.rs              # Dataset CRUD, S3 preview, presigned URLs
│   ├── repositories/
│   │   └── dataset_repo.rs                 # Dataset SQL queries (tenant-scoped)
│   └── dto/
│       ├── dataset.rs                      # DatasetResponse DTO
│       └── pipeline.rs                     # Pipeline trigger/status DTOs
│
├── apps/workers/src/
│   ├── clients.py                          # Shared S3/DB/Redis clients (module-level singletons)
│   ├── s3_paths.py                         # Python S3 path builders (mirrors Rust s3_paths.rs)
│   └── activities/
│       ├── parse_document.py               # Document parsing (PDF, DOCX, HTML, MD, TXT, CSV)
│       ├── chunk_text.py                   # Recursive text chunking with configurable overlap
│       ├── generate_pairs.py               # LLM-powered synthetic pair generation
│       └── build_dataset.py                # Quality filtering, ChatML formatting, train/val split
│
├── apps/web/src/
│   ├── hooks/
│   │   ├── use-documents.ts                # Document list + upload hooks with polling support
│   │   ├── use-pipeline.ts                 # Pipeline status + trigger hooks with smart polling
│   │   └── use-datasets.ts                 # Dataset list, detail, preview hooks
│   └── app/(dashboard)/projects/[id]/
│       ├── page.tsx                        # Rewritten: upload, doc list, pipeline status, actions
│       └── dataset/page.tsx                # New: dataset review with ChatML pair rendering
```

**Modified files** (existing Phase 0 files updated):

| File | Change |
|---|---|
| `apps/workers/src/config.py` | Added LLM API config (key, base_url, model, max_tokens) |
| `apps/workers/src/worker.py` | Initialize infrastructure clients, register new activities |
| `apps/workers/pyproject.toml` | Added parsing, DB, S3, LLM deps (pymupdf, asyncpg, boto3, httpx, etc.) |
| `apps/workers/src/workflows/ingest.py` | Wired to real parse_document + get_document_info activities |
| `apps/workers/src/workflows/refine.py` | Chains: chunk_text -> generate_pairs -> build_dataset |
| `apps/workers/src/workflows/full_pipeline.py` | Fixed to use RefineWorkflow result directly (was broken) |
| `crates/api/src/app_state.rs` | Added `temporal: Option<TemporalClient>`, accessor method |
| `crates/api/src/config.rs` | Activated temporal config fields (removed dead_code allow) |
| `crates/api/src/main.rs` | Added `mod temporal` declaration |
| `crates/api/src/routes/mod.rs` | Registered pipeline + datasets routers |
| `crates/api/src/services/mod.rs` | Added pipeline_service + dataset_service modules |
| `crates/api/src/repositories/mod.rs` | Added dataset_repo module |
| `crates/api/src/dto/mod.rs` | Added dataset + pipeline modules |
| `apps/web/src/lib/api-client.ts` | Added Document, Dataset, Pipeline types + API methods |

---

## Document Parsing (6 Formats)

`apps/workers/src/activities/parse_document.py` — CPU-only, no GPU dependencies.

| Format | Library | Strategy |
|---|---|---|
| PDF | PyMuPDF (`fitz`) | Extract text blocks, detect headings by font size (>14pt) |
| DOCX | `python-docx` | Paragraphs, headings (by Word style), tables |
| HTML | `BeautifulSoup` | Strip script/style, extract h1-h6/p/li/td elements |
| Markdown | `markdown` + BS4 | Convert to HTML first, then parse with BeautifulSoup |
| TXT | stdlib | Split by double newlines into paragraphs |
| CSV | stdlib `csv` | Structured tabular format with headers + rows |

**Parse flow:**
1. Idempotency check — skip if document status is already `parsed`
2. Update DB status to `parsing`
3. Download raw file from S3 via `storage_path`
4. Route to parser by `mime_type` (with extension fallback)
5. Detect language via `langdetect` (first 5000 chars)
6. Compute quality score (0.0-1.0): text density + structure detection + encoding validity
7. Build structured JSON: `{doc_id, pages: [{page_num, text, sections: [{type, content}]}]}`
8. Upload parsed JSON to S3: `parsed/{tenant_id}/{project_id}/{doc_id}.json`
9. Update DB: `status = "parsed"`, `parse_quality`, `page_count`, `language`
10. On error: `status = "failed"`, `error_message` (truncated to 500 chars)

**Activity heartbeats** at 3 points: downloading, parsing, uploading_result.

---

## Data Refinement Pipeline

Three activities chained by `RefineWorkflow`:

### 1. Chunk Text (`chunk_text`)

Recursive splitting strategy: paragraphs first, then sentences for oversized paragraphs.

- Default: 1500 chars per chunk, 200 char overlap
- Downloads parsed JSONs from S3 per document
- Outputs JSONL to S3: `chunks/{tenant_id}/{project_id}/{batch_id}.jsonl`
- Each chunk record: `{chunk_id, doc_id, page_num, chunk_index, text, char_count}`

### 2. Generate Synthetic Pairs (`generate_synthetic_pairs`)

Uses a configurable LLM API (OpenAI-compatible format) to create instruction/response pairs.

- **Provider-agnostic**: works with any OpenAI-compatible API (OpenAI, Claude, Groq, Ollama, etc.)
- **Config via env vars**: `APP_LLM_API_KEY`, `APP_LLM_API_BASE_URL`, `APP_LLM_MODEL`, `APP_LLM_MAX_TOKENS`
- **3 prompt templates** by task type:
  - `question_answering` — factual, inferential, comparative Q&A
  - `instruction_following` — summarize, explain, extract, compare tasks
  - `reasoning` — complex analysis with step-by-step reasoning
- Skips chunks < 50 chars, limits context to 3000 chars per chunk
- Graceful degradation: logs warning on LLM failures, continues with remaining chunks
- Outputs JSONL to S3: `pairs/{tenant_id}/{project_id}/{batch_id}.jsonl`

### 3. Build Dataset (`build_dataset`)

Assembles pairs into a training-ready ChatML dataset.

- **Quality filtering**: removes empty, too-short (<20 chars response, <10 chars instruction), too-long (>5000 chars)
- **Deduplication**: MD5 content hash on `instruction|response`
- **ChatML format**: `{"messages": [{"role": "system/user/assistant", "content": "..."}]}`
- **Train/val split**: 90% train, 10% validation (separate files)
- Creates/updates dataset record in DB: `status = "review_pending"`, `pair_count`, stats JSON
- Outputs to S3: `datasets/{tenant_id}/{project_id}/{dataset_id}.jsonl`

---

## Temporal Workflows (Implemented)

| Workflow | Status | Stages | Error Handling |
|---|---|---|---|
| `IngestWorkflow` | Implemented | `get_document_info` → `parse_document` per doc | Partial failure tolerant (3/5 succeed = OK) |
| `RefineWorkflow` | Implemented | `chunk_text` → `generate_pairs` → `build_dataset` | Early exit on zero chunks/pairs |
| `FullPipelineWorkflow` | Fixed | Ingest → Refine → Train → Evaluate → Deploy | Uses RefineWorkflow result directly |

**Key fix in FullPipelineWorkflow**: Removed a redundant `build_dataset` call that had wrong parameters. `RefineWorkflow` already builds the dataset internally — the pipeline now uses `refine_result` directly for downstream stages.

---

## Rust Temporal Client

`crates/api/src/temporal.rs` — HTTP-based, no gRPC dependency.

| Feature | Detail |
|---|---|
| Protocol | Temporal HTTP API (v1.24+) via `reqwest` |
| Payload encoding | Base64-encoded JSON (Temporal wire format) |
| Methods | `start_ingest()`, `start_refine()`, `get_workflow_status()` |
| Failure mode | `Option<TemporalClient>` in AppState — API runs fine without Temporal |

The client is intentionally lightweight. It encodes workflow arguments as base64 JSON payloads matching Temporal's HTTP API format. This avoids the heavyweight `tonic`/gRPC dependency that the official Temporal Rust SDK would require.

---

## API Endpoints (Phase 1 Additions)

| Method | Path | Handler | Purpose |
|---|---|---|---|
| `POST` | `/api/v1/projects/:id/parse` | `pipeline::trigger_parse` | Start IngestWorkflow for unparsed docs |
| `POST` | `/api/v1/projects/:id/refine` | `pipeline::trigger_refine` | Start RefineWorkflow for parsed docs |
| `GET` | `/api/v1/projects/:id/status` | `pipeline::get_status` | Aggregate pipeline status counts |
| `GET` | `/api/v1/projects/:id/datasets` | `datasets::list_datasets` | List datasets (paginated) |
| `GET` | `/api/v1/datasets/:id` | `datasets::get_dataset` | Get single dataset |
| `GET` | `/api/v1/datasets/:id/preview` | `datasets::preview_dataset` | Preview first N rows from JSONL |
| `GET` | `/api/v1/documents/:id/parsed` | `datasets::get_parsed_content` | Presigned URL for parsed JSON |

**Pipeline status** uses `tokio::try_join!` for 9 concurrent DB count queries — returns document counts by status (uploaded, parsing, parsed, failed) and dataset counts by status (generating, review_pending, approved).

**Dataset preview** reads the JSONL file from S3 via `ObjectStorage` trait, parses first N lines as JSON, and returns them directly — no DB round-trip for the data itself.

All routes follow the Phase 0 pattern: thin handler → service (business logic) → repository (SQL with tenant_id).

---

## Frontend Pages

### Project Detail (Rewritten)

`apps/web/src/app/(dashboard)/projects/[id]/page.tsx`

| Feature | Implementation |
|---|---|
| File upload | Drag-and-drop area with `onDragOver`/`onDragLeave`/`onDrop` events |
| Upload method | `FormData` + `fetch` with Clerk auth token (no Content-Type header) |
| Document list | Status badges (color-coded), file size, language, page count, parse quality |
| Pipeline status | 4 count cards: uploaded / parsing / parsed / failed |
| Action buttons | "Parse Documents" (POST /parse), "Generate Training Data" (POST /refine) |
| Dataset list | Links to dataset review page with pair count and status |
| Live updates | Conditional polling (3s) when pipeline is active, stops when idle |

### Dataset Review (New)

`apps/web/src/app/(dashboard)/projects/[id]/dataset/page.tsx`

- Dataset header with status badge and pair count
- Statistics grid: total pairs, train/val split, filtered count
- ChatML pair preview: renders system/user/assistant messages in styled cards
- Source document reference per pair

### React Query Hooks

| Hook | File | Features |
|---|---|---|
| `useDocuments` | `use-documents.ts` | Paginated list with optional polling interval |
| `useUploadDocuments` | `use-documents.ts` | File upload mutation, invalidates docs + status caches |
| `usePipelineStatus` | `use-pipeline.ts` | Smart polling: 3s when active, stops when idle |
| `useTriggerParse` | `use-pipeline.ts` | Mutation to start document parsing |
| `useTriggerRefine` | `use-pipeline.ts` | Mutation with task_type + config params |
| `useDatasets` | `use-datasets.ts` | Paginated dataset list |
| `useDataset` | `use-datasets.ts` | Single dataset by ID |
| `useDatasetPreview` | `use-datasets.ts` | Preview rows with configurable max |

---

## Worker Infrastructure

### Shared Clients (`clients.py`)

Module-level singletons initialized once at worker startup:

| Client | Library | Purpose |
|---|---|---|
| S3 | `boto3` | Object storage (synchronous — fine for Temporal thread pool) |
| PostgreSQL | `asyncpg` | Database queries (pool: 2-10 connections) |
| Redis | `redis.asyncio` | Caching (used by future phases) |

Accessor functions (`get_s3()`, `get_db()`, `get_redis()`, `get_settings()`) raise `RuntimeError` if called before initialization — never silently return `None`.

### S3 Path Builders (`s3_paths.py`)

Mirrors `crates/shared/src/s3_paths.rs` exactly:

| Function | Path Pattern |
|---|---|
| `upload_path()` | `uploads/{tenant}/{project}/{file}.{ext}` |
| `parsed_path()` | `parsed/{tenant}/{project}/{doc}.json` |
| `chunks_path()` | `chunks/{tenant}/{project}/{batch}.jsonl` |
| `pairs_path()` | `pairs/{tenant}/{project}/{batch}.jsonl` |
| `dataset_path()` | `datasets/{tenant}/{project}/{dataset}.jsonl` |
| `adapter_prefix()` | `adapters/{tenant}/{model}/` |
| `adapter_file()` | `adapters/{tenant}/{model}/{filename}` |
| `checkpoint_prefix()` | `checkpoints/{tenant}/{training}/` |
| `export_path()` | `exports/{tenant}/{model}/{filename}` |

### Python Dependencies Added

| Package | Version | Purpose |
|---|---|---|
| `pymupdf` | >=1.24 | PDF text extraction |
| `python-docx` | >=1.1 | DOCX parsing |
| `beautifulsoup4` | >=4.12 | HTML text extraction |
| `markdown` | >=3.6 | Markdown → HTML conversion |
| `langdetect` | >=1.0.9 | Language detection (ISO 639-1) |
| `asyncpg` | >=0.29 | PostgreSQL async driver |
| `boto3` | >=1.35 | S3-compatible object storage |
| `httpx` | >=0.27 | Async HTTP client for LLM API calls |

---

## Status Polling

### Frontend (React Query)

`usePipelineStatus` hook with dynamic `refetchInterval`:

```typescript
refetchInterval: (query) => {
    const data = query.state.data;
    if (!data) return false;
    const isActive = data.documents.parsing > 0 || data.datasets.generating > 0;
    return isActive ? 3000 : false;
},
```

- Polls every 3s while documents are parsing or datasets are generating
- Stops polling automatically when pipeline is idle
- `useDocuments` also supports an optional `pollingInterval` parameter

### Python Workers (Temporal Heartbeats)

| Activity | Heartbeat Points |
|---|---|
| `parse_document` | downloading, parsing, uploading_result |
| `chunk_text` | Per document: `chunking {doc_id}` |
| `generate_pairs` | Per chunk (default heartbeat) |

Heartbeats enable Temporal to detect stuck activities and retry them per the retry policy.

---

## Key Architectural Decisions

1. **CPU-only parsing** — Used PyMuPDF instead of MinerU for PDF parsing. MinerU requires GPU and complex setup. PyMuPDF handles text extraction well for MVP; MinerU can be swapped in later for OCR-heavy PDFs.

2. **Provider-agnostic LLM API** — Synthetic pair generation uses OpenAI-compatible format, configurable via env vars. Works with OpenAI, Claude (via compatibility endpoint), Groq, Ollama, or any provider that speaks the same protocol. No vendor lock-in.

3. **HTTP-based Temporal client** — Used Temporal's HTTP API via `reqwest` instead of the gRPC-based Rust SDK. This avoids pulling in `tonic`, `prost`, and Protocol Buffers compilation — saving ~50 crate dependencies. The tradeoff is less type safety, but the 3 methods we need (start_ingest, start_refine, get_status) are simple enough.

4. **Optional Temporal** — `Option<TemporalClient>` in AppState means the API server runs fine without Temporal. In dev mode, you can test API endpoints without the workflow engine running.

5. **Module-level client singletons** — Python workers initialize S3/DB/Redis once at startup via `init_clients()`. Activities access them via module-level getters. This avoids Temporal's activity context limitations (can't pass complex objects) and ensures connection pooling.

6. **Smart polling over WebSockets** — Used React Query's `refetchInterval` with a conditional callback instead of WebSocket connections. Simpler to implement, works through proxies/CDNs, and 3s polling is more than adequate for pipeline stage transitions that take minutes.

7. **RefineWorkflow owns dataset building** — The refine workflow chains chunk → generate → build_dataset internally. FullPipelineWorkflow calls RefineWorkflow as a child workflow and uses the result directly, rather than calling build_dataset separately. Single responsibility.

---

## Build Steps Completed

| # | Step | Status |
|---|---|---|
| 1 | Python worker infrastructure (clients.py, s3_paths.py, config, deps) | Done |
| 2 | `parse_document` activity (PDF, DOCX, HTML, MD, TXT, CSV) | Done |
| 3 | IngestWorkflow wiring (get_document_info + parse per doc) | Done |
| 4 | Rust Temporal client (HTTP API, start_ingest, start_refine) | Done |
| 5 | New API endpoints (7 routes: pipeline triggers + dataset CRUD) | Done |
| 6 | Synthetic data generation (chunk_text, generate_pairs, build_dataset) | Done |
| 7 | Frontend (upload UI, pipeline status, dataset review page) | Done |
| 8 | Status polling (React Query refetchInterval, activity heartbeats) | Done |

---

## Verification

- `cargo clippy -- -D warnings` — zero warnings
- `cargo test --workspace` — 20 tests pass
- `uv run ruff check src/` — Python lint clean
- `uv run ruff format --check src/` — Python formatting clean
- `pnpm --filter @platform/web type-check` — TypeScript clean (`tsc --noEmit`)
- `pnpm --filter @platform/web lint` — ESLint clean

---

## What's Next — Phase 2 (Training Engine)

Phase 2 is where fine-tuning happens: take the datasets built in Phase 1 and train LoRA/QLoRA adapters.

**Key deliverables**:
- Unsloth/TRL integration for fast LoRA/QLoRA fine-tuning
- Support for SFT, DPO, and ORPO training methods
- Hyperparameter management (learning rate, epochs, rank, etc.)
- Checkpoint management (save/resume from S3)
- GPU class selection and cost estimation
- Real-time training metrics streaming via Redis
- Frontend: training dashboard with live loss curves, progress bar, cost tracking

**Infrastructure from Phase 1 that Phase 2 builds on**:
- Dataset assembly pipeline (ChatML format) -> ready to feed to trainer
- S3 path builders for adapters/checkpoints -> already defined
- Temporal TrainWorkflow stub -> signature ready, fill in Unsloth code
- Activity heartbeats pattern -> extend for training progress reporting
