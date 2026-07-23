# Feature Guide

User-facing features, what they're for, and the technical details behind them.
Each section covers: purpose, how to use it, API surface, and implementation
notes for developers.

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
