# Feature Guide

User-facing features, what they're for, and the technical details behind them.
Each section covers: purpose, how to use it, API surface, and implementation
notes for developers.

---

## Document-Grounded Eval Builder (Golden Holdout)

**Purpose:** answer "does the fine-tuned model actually know MY documents?"
with data the model has provably never trained on. During synthetic data
generation, a slice of document chunks is held out; eval Q&A pairs generated
from those chunks become the model's **golden set**.

### How it works

- During refine/generation, `golden_holdout_ratio` (default 0.1) of chunks are
  reserved — deterministically, by content-addressed fingerprint (sha256 of
  `doc_id:chunk_id`), so re-runs hold out the same chunks. Holdout is skipped
  below 10 chunks and capped at 25% of the corpus.
- Golden Q&A pairs (3 per held-out chunk) are generated with base facets and
  always faithfulness-gated, then stored next to the dataset as
  `<dataset>_golden.jsonl`. The dataset's stats record `golden_pairs`.
- Evaluation loads the golden set by that path convention. The
  **Document Knowledge** suite (weight 0.30) asks both the fine-tuned model
  and its base model the golden questions, scores both with the judge, and
  reports `knowledge_lift` = fine-tuned mean − base mean. Positive lift means
  the model genuinely absorbed document knowledge rather than general ability.
- The evaluation page shows the lift headline plus base-vs-tuned score cards;
  a non-positive lift adds a recommendation to revisit the dataset.

### Files

`apps/workers/src/activities/generate_pairs.py` (holdout selection),
`build_dataset.py` (golden write-out), `run_evaluation.py`
(`DocumentKnowledgeSuite`), `crates/shared/src/types.rs`
(`DocKnowledgeScores`).

---

## Eval-Gated Deployment

**Purpose:** stop bad models from reaching production. A deploy is blocked
unless the model's latest **completed** evaluation clears configured
thresholds.

### How to use

Set either or both (unset = that rule disabled):

```bash
DEPLOY_MIN_AB_WIN_RATE=0.5           # A/B win rate vs base must be >= this
DEPLOY_MAX_BENCHMARK_REGRESSION=10   # block if benchmark regression > this many points
```

A failing (or missing) evaluation returns **409 Conflict** with the reason —
the gate **fails closed**: no completed eval means no deploy. Rollbacks to a
previously deployed version bypass the gate (they were vetted when first
deployed). Implementation: `crates/api/src/services/deploy_gate.rs`, invoked
from the deployment service; metrics are read from the evaluation's raw
scores JSON.

---

## JSONL Dataset Import (OpenAI Chat Format)

**Purpose:** bring existing training data instead of (or alongside)
generating it from documents.

### How to use

`POST /api/v1/projects/{id}/datasets/import` (multipart: `file` + optional
`name`), or the dashboard's import action. Each line is an OpenAI chat sample
(`{"messages": [...]}`, optional top-level `tools`). Malformed rows are
reported per row (line number + reason, capped at 100 detailed errors) rather
than failing the file. The dataset enters the standard `review_pending`
approve/reject flow.

**Tool-calling is preserved on purpose**: `role: "tool"` turns, assistant
`tool_calls`, `tool_call_id`, and the `tools` array are validated and carried
through verbatim into the stored records — groundwork for the agent/tool-call
fine-tuning track. Parser: `crates/api/src/services/jsonl_import.rs` (pure,
fully unit-tested); storage via `DatasetService::store_records_as_dataset`
(90/10 train/val split).

---

## Ollama Export

**Purpose:** run your fine-tuned model locally with one command.

### How to use

1. Export the model as GGUF (`POST /api/v1/models/{id}/exports`,
   `quant_type` one of Q4_K_M / Q5_K_M / Q6_K / Q8_0).
2. `GET /api/v1/exports/{id}/ollama` returns a ready-made recipe: a
   `Modelfile` (FROM the GGUF; the deployment system prompt as `SYSTEM` when
   present) plus the exact `ollama create` / `ollama run` commands.

The GGUF embeds the chat template from training, so the Modelfile needs no
`TEMPLATE` block. Generated model names use a generic `finetuned-` prefix.
Implementation: `crates/api/src/services/ollama_modelfile.rs`.

---

## Facet Subtopic Expansion (Generation Diversity)

**Purpose:** prevent synthetic samples from clustering on each facet's most
obvious phrasing. Each document-grounded facet is expanded into up to
`facet_subtopics` (default 3, cap 6) narrower subtopics — grounded in an
excerpt sampled **across** the document, not just its head — and generation
rotates chunks over the flattened facet×subtopic angles.

Strictly additive and best-effort: expansion failure falls back to the base
facet, `facet_subtopics: 0` disables it, expansion is skipped when chunks
don't outnumber facets, and the golden eval set deliberately keeps base
facets only. Implementation: `FacetExpander` protocol + `LlmFacetExpander`
(`apps/workers/src/datagen/`), wiring in `generate_pairs.py`
(`facets_to_angles`, `doc_sample_for_expansion`).

---

## Data Flywheel — Traffic Capture & Feedback

**Purpose:** close the loop between serving and training. Once a model is
deployed, its real production traffic is the best source of training signal:
capture requests/responses, let end-users and the team flag weak answers, and
feed the curated examples into the next training run.

Stage 1 is capture + feedback; stage 2 is promotion of rated samples into
training datasets. Together they close the loop: capture → rate → correct →
promote → approve dataset → retrain.

### How to use

1. **Enable capture** on a deployed model: model page → *Production Feedback →
   Review Samples* → toggle **Capture: On** (or `PUT /api/v1/models/{id}/capture`
   with `{"enabled": true}`). Capture is **off by default** — production
   prompts can contain sensitive data, so recording is an explicit tenant
   decision. The setting survives undeploy/redeploy.
2. **Send traffic** through the OpenAI-compatible API as usual. Every captured
   completion returns an `x-sample-id` response header (both streaming and
   non-streaming).
3. **Collect end-user feedback** from your application:

   ```bash
   curl -X POST $API/v1/feedback \
     -H "Authorization: Bearer $API_KEY" \
     -d '{"sample_id": "<x-sample-id>", "rating": "negative", "comment": "wrong policy quoted"}'
   ```

   Ratings are `positive` | `negative`. The API key must belong to the model
   that produced the sample.
4. **Review in the dashboard**: the model's *Feedback* page lists captured
   samples newest-first with filters (All / Unrated / Positive / Negative),
   expandable full conversations, and 👍/👎 rating buttons for the team.
5. **Promote to training data** (stage 2): select samples on the Feedback page
   and click *Promote N to Training Data*. Negative-rated samples require a
   **corrected response** first (promoting a response the user flagged as bad
   would poison the training set); positive/unrated samples use the captured
   response as-is unless a correction is provided. Promotion creates a new
   dataset in the model's project — named `Production Feedback — {model}` by
   default — in the standard `review_pending` flow: preview it, approve it,
   and train. Each sample can be promoted once (`promoted` badge afterwards).

### API surface

| Endpoint | Auth | Purpose |
|---|---|---|
| `PUT /api/v1/models/{id}/capture` | JWT (Member+) | Toggle capture |
| `GET /api/v1/models/{id}/samples?rating=&offset=&limit=` | JWT | List captured samples (`rating` = `positive`, `negative`, or `unrated`) |
| `POST /api/v1/samples/{id}/feedback` | JWT (Member+) | Team rating from the dashboard |
| `POST /api/v1/models/{id}/samples/promote` | JWT (Member+) | Create a training dataset from selected samples (≤500 per call) |
| `POST /v1/feedback` | API key | End-user rating from the tenant's own app |

### Implementation notes

- **Storage:** `inference_samples` table (migration 022) — tenant-scoped with
  RLS, `messages` JSONB (exactly what the model saw, including the injected
  deployment system prompt), `response` text, optional `rating`/`rating_comment`.
  The capture flag is `models.capture_traffic` (column, not
  `deployment_config` — the config blob is rebuilt on every deploy).
- **Capture is strictly best-effort** (`InferenceSampleService::capture_best_effort`):
  a failed write logs a warning and never fails or slows the inference request.
  This is deliberately the opposite of billing, which uses durable
  reservation/finalize — losing a flywheel sample costs nothing.
- **Non-streaming path:** completion text is read from `choices[0].message.content`
  after billing, then the row is inserted before the response returns.
- **Streaming path:** the SSE tee that already scans for the usage chunk also
  accumulates `choices[0].delta.content` fragments. The billing finalizer task
  inserts the sample after the usage chunk arrives (which is the final data
  chunk, so the accumulator is complete). If the client disconnects
  mid-stream, the partial response is **not** captured — a truncated answer is
  not a usable training example.
- **Size cap:** samples over 256 KiB (messages + response) are skipped, not
  truncated — cut-off JSON is worthless for training.
- **Sample IDs are minted before the response** so `x-sample-id` can be set on
  streaming responses (headers go out before the body).
- **Scope guards:** `/v1/feedback` verifies the sample belongs to the API
  key's model; dashboard routes are tenant-scoped through the repository layer
  (every query carries `tenant_id`) plus RLS as the second layer.
- **Batch endpoint** (`/v1/chat/completions/batch`) does not capture — batch
  callers get no per-item sample id to rate against.
- **Promotion** builds records as the captured conversation plus the
  (possibly corrected) response appended as the final assistant turn, with
  `metadata: {source: "production_feedback", sample_id}` for lineage. Records
  are stored through the same path as JSONL imports
  (`DatasetService::store_records_as_dataset`): 90/10 train/val split, JSONL
  in object storage, `review_pending` dataset row with provenance stats
  (`source`, `model_id`). `inference_samples.promoted_at` (migration 024)
  guards against double-promotion.

### Files

- `crates/db/src/migrations/022_inference_samples.sql`
- `crates/api/src/repositories/inference_sample_repo.rs`
- `crates/api/src/services/inference_sample_service.rs`
- `crates/api/src/routes/feedback.rs` (dashboard + `/v1/feedback` routers)
- `crates/api/src/routes/inference.rs` (capture wiring)
- `apps/web/src/hooks/use-feedback.ts`,
  `apps/web/src/app/(dashboard)/projects/[id]/models/[modelId]/feedback/page.tsx`

## Realtime Transports (SSE + WebSocket)

Live updates ship over two transports:

- **SSE** (what the dashboard uses): `GET .../status/stream` and
  `GET .../training-jobs/{id}/metrics/stream` consumed by
  `use-status-stream.ts` / `use-training-metrics.ts` via `fetch()` +
  ReadableStream (Bearer auth; EventSource can't set headers). Server sends
  events only on change; clients reconnect with exponential backoff.
- **WebSocket** (`GET /api/v1/ws?token=...`): subscribe/unsubscribe message
  protocol tailing Redis streams. Auth via query-param token (browsers can't
  set WS headers); only `training:{job_id}` channels, authorized against the
  caller's tenant before any stream is tailed. Currently unused by the
  dashboard — kept as the transport for future push channels (notifications,
  deploy status) and external consumers.

### Files

- `crates/api/src/routes/ws.rs`
- `apps/web/src/hooks/use-status-stream.ts`, `use-training-metrics.ts`
