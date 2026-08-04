# Distillation Stage 2 — Local Testing Guide

> How to verify the high-fidelity (logit/KL) distillation feature end to end.
> Based on the implementation plan's §4 checklist, corrected against what was
> actually built — where the two differ, this file is right and the plan is not.
> Stage 1's guide still applies to everything text-distillation:
> [STAGE1-TESTING.md](STAGE1-TESTING.md). Measured vLLM/tokenizer facts are in
> [STAGE2-SPIKE-FINDINGS.md](STAGE2-SPIKE-FINDINGS.md) and are not repeated here.

**Three corrections to the plan, up front:**

1. **Hosted teachers are not in the teacher picker.** A hosted teacher scores; it
   never generates. The upgrade is offered on a dataset a hosted-catalog teacher
   already wrote, inside the **Start Training** form.
2. **The eligible students are the upstream `Qwen/*` catalog entries**, never the
   `unsloth/*` re-uploads — those measurably fail the tokenizer guard.
3. **Nothing deletes artifacts on a timer.** There is no
   `TEACHER_ARTIFACT_RETENTION_HOURS` and no cleanup activity, so the plan's
   retention step is not testable. Tenant erasure does remove them.

---

## 1. What you need before starting

| Requirement | Why | Notes |
| --- | --- | --- |
| A **teacher-generated dataset** whose teacher is a hosted catalog model | The upgrade offer is derived from `datasets.config.teacher.model`. Built-in hosted teachers: `Qwen/Qwen3-32B`, `Qwen/Qwen2.5-32B-Instruct`. | Generate it Stage-1 style ("Distill a Larger Model") against an endpoint serving one of those exact model names. The teacher's *name* is what the offer matches on. |
| An **eligible student** | The pair must share a byte-identical tokenizer. | `Qwen/Qwen3-8B` or `Qwen/Qwen3-4B` (for a Qwen3-32B dataset), `Qwen/Qwen2.5-7B-Instruct` (for a Qwen2.5-32B dataset). All three are in the model catalog, labelled *(distillable)*. |
| A GPU path for the **teacher scoring pass** | The teacher runs under vLLM on our own GPU, at the catalog's `gpu_class` (`a10080gb` for both built-ins). | Modal, or a local A100-80GB. This is the only genuinely new GPU workload. |
| A GPU path for training + evaluation | Unchanged from every other mode. | Same GPU as any quick-mode run. |
| A judge | The parity suite still scores through the tenant LLM judge. | Covered by tenant LLM settings. |

Things that will NOT work — verify they fail cleanly rather than proceeding:

- An **imported** or non-teacher dataset → no upgrade offer at all.
- A teacher only reachable as an API (not in the hosted catalog) → offer replaced
  by *"Higher-fidelity training needs a teacher we can run ourselves, and this one
  is only reachable as an API. Standard distillation is unaffected."*
- An **`unsloth/*` student** against a hosted teacher → the tokenizer message
  (§5.1).
- `distill_method` or `teacher_artifacts_prefix` sent as **hyperparams** → job
  refused: *"These hyperparams are set by the platform, not per job: …"*.

## 2. Start the stack

```bash
make infra          # PostgreSQL, Redis, MinIO
make temporal       # Temporal server
make migrate        # must report 31 migrations (031 = teacher-extraction billing)
make dev-api        # Rust API
make dev-workers    # Python Temporal worker (needs the GPU env)
make dev-web        # dashboard
```

Optional for the cap test (§5.2), set before starting the API:

```bash
TEACHER_GPU_SPEND_CAP_STARTER=0.01   # any admission then refuses
```

## 3. Happy path (UI)

1. Produce the teacher-generated dataset first (Stage 1 flow), approve it.
2. Project page → **Start Training**.
3. Mode → **Distill (teacher-generated dataset)**, then pick that dataset.
4. The **"Higher-fidelity training available"** card appears, naming the teacher
   and offering two radios:
   - *Standard — the teacher's answers* (default, no extra GPU time)
   - *Higher fidelity — adds about $X.XX of GPU time*
5. Choose higher fidelity. Optionally open **Fidelity details** to change
   *"How much of the teacher's confidence to keep"* (default 32) or *"How the
   teacher runs while it scores"* (default *Full — recommended*).
6. Student model → one of the *(distillable)* entries above.
7. Start the job.

Expect, in order: a **"Scoring with teacher"** chip on the pipeline row, then the
normal training progress, then evaluation.

## 4. What to verify

1. **The offer is honest about its own precision.** The card shows a cost
   estimate and, because dataset token counts are never populated (see §6), also
   the caveat that the price is worked out from the number of examples rather than
   a measured word count. An ineligible pair shows a one-line reason and no
   radios — never a greyed-out control.
2. **Admission priced the run.** `training_jobs.teacher_config` contains an
   `extraction` block (teacher model, pinned revision, precision, `top_k_logprobs`,
   `gpu_class`); `teacher_extraction_status` moves `running` → `completed`.
3. **Artifacts exist and are self-describing.** Under
   `datasets/{tenant}/{project}/{dataset}-teacher-logprobs/{digest}/`: one or more
   `shard-*.npz` plus `manifest.json` written last. The manifest carries the
   teacher model + revision, `top_k`/`width`, `tokenizer_hash`,
   `rendering_fingerprint`, `vllm_version` and totals.
4. **Resume works.** Kill the worker mid-scoring and let Temporal retry: the run
   picks up from the last completed shard (`progress/*.json` markers), it does not
   restart. The markers are deleted when the manifest commits.
5. **Re-running is free.** Start the same dataset + teacher + `top_k` again →
   the committed manifest is reused, the GPU is never provisioned, metrics report
   `reused_artifacts`. This is the cheapest way to test everything downstream.
6. **Billing.** One `extraction` billing row per run: reserved when scoring
   starts, finalized from measured runtime when it ends. `teacher_extraction_cost`
   is set on the job and is separate from the training run's `actual_cost`.
7. **Training actually consumed the distributions.** Job metrics contain
   `distill_logit_train_loss`, `distill_logit_train_steps`,
   `distill_logit_scored_positions`, `kd_alpha`, `ce_alpha`, `kd_temperature`,
   `tail_beta`, and `teacher_artifacts_prefix`. Loss decreases; an adapter is
   exported like any other run.
8. **Fidelity is reported.** The evaluation's `teacher_parity` scores contain
   `teacher_student_kl` alongside the Stage 1 keys. The evaluation page shows
   **Distribution match** inside the parity panel with *"Lower is better"* next to
   the value, and the model page shows the matching tile. It is report-only: the
   overall score does not move, and the deploy gate ignores it.
9. **The metric never breaks an evaluation.** Rename or delete the shard files (or
   the whole prefix) and re-run the evaluation: the four other suites and parity
   still complete, and `teacher_student_kl` is simply absent. Same for a
   `manifest.json` whose `tokenizer_hash` you edit by hand — the mismatch is
   logged and skipped, never raised.

## 5. Negative paths

### 5.1 Tokenizer mismatch

Pick `unsloth/Qwen2.5-7B-Instruct` as the student for a `Qwen2.5-32B-Instruct`
dataset. The eligibility call returns ineligible, and forcing the job through the
API refuses it with, verbatim:

> These two models read text differently, so high-fidelity training is not
> possible between them. Standard distillation works — switch and re-run.

The same string is raised non-retryably by the extraction preflight *and* by
training when it re-checks the manifest, so a catalog edit between the two passes
fails the run rather than shifting every target. Forcing that race: start a
fidelity run, let it commit artifacts, then edit the catalog entry's revision and
start a second run against them.

### 5.2 Spend cap

With `TEACHER_GPU_SPEND_CAP_STARTER=0.01`, admission refuses with HTTP 403:

> This run reached your GPU spending cap for teachers. Raise the cap in
> Settings → Billing or resume with a smaller dataset.

Two honest limits worth knowing while testing: the cap sums **delivered**
`extraction` billing rows for the month, so runs admitted close together can
jointly exceed it, and a run already scoring is never stopped part-way. Standard
(text) distillation is unaffected by the cap.

### 5.3 Nothing to score

A dataset whose examples all fail rendering fails fast with *"None of this
dataset's examples could be scored, so there is nothing to train on at higher
fidelity. Standard distillation still works."*

## 6. Known gaps (do not treat as bugs)

- **Dataset token counts are never measured.** The columns exist and the
  estimator prefers them, but nothing writes them, so every estimate is the
  `approximate` basis (examples × 200 tokens) and the UI says so.
- **Extraction status is not in the API.** `teacher_extraction_status` is written
  to the database but not returned, so the "Scoring with teacher" chip only
  survives in the session that launched the run. Reloading the page loses it;
  scoring progress is still visible on the job's metrics stream as
  `stage: teacher_extraction`.
- **Artifacts are never reclaimed.** No retention window, no cleanup activity, and
  no dataset-delete route. They are removed only by tenant erasure
  (`DELETE /api/v1/admin/tenants/{id}`), which deletes the whole dataset prefix.
- **The spend cap is admission-only** — see §5.2.
- **`kd_alpha` / `ce_alpha` are not exposed in the UI.** They are hyperparams
  with measured defaults (0.9 / 0.1); only `top_k` and precision are on the form.

## 7. API-level smoke (optional, no UI)

```bash
# Eligibility + cost estimate. Always 200: an ineligible pair carries a reason.
curl -s -X POST localhost:8000/api/v1/teachers/cost-estimate \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"dataset_id":"'$DATASET_ID'","student_model":"Qwen/Qwen3-8B","top_k_logprobs":32}'
# → {"eligible":true,"teacher_model":"Qwen/Qwen3-32B",
#    "estimate":{"scored_tokens":…,"est_gpu_hours":…,"est_cost_usd":…,
#                "basis":"approximate","gpu_class":"a10080gb"}}

# The negative case: a re-upload student against the same dataset.
curl -s -X POST localhost:8000/api/v1/teachers/cost-estimate \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"dataset_id":"'$DATASET_ID'","student_model":"unsloth/Qwen2.5-7B-Instruct"}'
# → {"eligible":false,"reason":"These two models read text differently, …"}

# Start a fidelity run.
curl -s -X POST localhost:8000/api/v1/projects/$PROJECT_ID/training-jobs \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{
    "dataset_id": "'$DATASET_ID'",
    "base_model": "Qwen/Qwen3-8B",
    "method": "qlora",
    "mode": "distill",
    "distill": {"method": "logit", "precision": "bf16", "top_k_logprobs": 32}
  }'

# Scoring progress (stage: teacher_extraction), then training's own metrics.
curl -s localhost:8000/api/v1/training-jobs/$JOB_ID/metrics \
  -H "Authorization: Bearer $TOKEN"
```

## 8. What needs a real GPU, and roughly what it costs

Everything in §3 needs GPUs; only §5.1's ineligibility check, §5.2's cap refusal
and the §7 estimate calls are free (they are refused or answered before any GPU
is provisioned). Both built-in hosted teachers run on `a10080gb`, billed at the
**$3.00/hr** built-in rate; the student's training and evaluation run on whatever
class you pick, at that class's rate (`t4` $0.80 → `h100` $4.50).

| Step | GPU | Rough cost |
| --- | --- | --- |
| Teacher scoring pass, small dataset (~1000 examples) | A100-80GB | **~$0.45** — 0.1 h of the estimate is a fixed startup allowance, so small runs are dominated by engine load |
| Teacher scoring pass, ~1.8M scored tokens | A100-80GB | **~$1.80** (0.5 h scoring + 0.1 h startup) |
| Student training (QLoRA, small dataset) | whatever you pick | same as any quick-mode run |
| Evaluation, including the fidelity metric | same class as training | the metric adds at most 32 short forward passes — noise next to the suites' generation |
| Re-run of an identical scoring pass | none | **$0** — the committed manifest is reused |

Cheapest useful sequence: pay for exactly one scoring pass, then re-run training
and evaluation against the reused artifacts as many times as the checks need.
