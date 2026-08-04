# Distillation Stage 2 — Implementation Plan (offline logit/KL)

> Implements [DESIGN-SPEC.md](DESIGN-SPEC.md) §6 on top of Stage 1 ([IMPLEMENTATION-PLAN.md](IMPLEMENTATION-PLAN.md)).
> Status: **architecture-firm, details-soft** — written before Stage 1 lands so Stage 1's interfaces stay forward-compatible. Items marked **[after-S1]** get finalized against the merged Stage 1 code before execution.
> Branch: `feat/distillation-stage2`. PRs only — the user merges.

---

## 0. What Stage 2 adds on top of Stage 1

Stage 1 gives us: teacher abstraction (+ policy/provenance/encryption), `distill` mode, teacher-driven datagen, `TeacherParitySuite`, parity UI. Stage 2 reuses **all** of it and adds exactly four new things:

| New piece | Why |
| --- | --- |
| **Hosted teacher** (`TeacherConfig.kind = "hosted"`) — open-weight teacher we run per-job on Modal vLLM, scale-to-zero | APIs can't provide `prompt_logprobs`; logit KD requires a white-box teacher |
| **Logprob extraction job** + artifact format | The teacher's top-k per-token distributions, precomputed once, stored, streamed at training |
| **`distill_logit` training strategy** — KL+CE loss on stored logprobs | The actual fidelity upgrade (dark knowledge) |
| **Teacher-GPU billing + per-tenant caps** | Stage 1 teacher spend was on the user's API key; hosted teachers burn *our* metered GPUs |

Everything else (mode, parity report, provenance, deploy gate) is unchanged Stage 1 machinery.

## 1. UX design

### Principle: fidelity is an upgrade offer, never a decision burden

The user still makes the same three decisions (teacher, student, go). Stage 2 appears as **one recommendation card** that shows up only when eligible — never a required choice, never jargon.

### The recommendation card (teacher step)

Shown only when: chosen teacher is a **catalog hosted** open-weight model AND student passes the tokenizer-identity check:

> **Higher-fidelity training available**
> Because this teacher is an open model in the same family as your student, we can train on its token-level answer *confidence* — not just its text. Usually noticeably closer to the teacher.
> ○ Standard (text) — default ● Higher fidelity (adds estimated **~$X.XX** of GPU time)

- Default stays **Standard** — upgrading is one click, opt-in (spec §8: recommended, never silent).
- The cost estimate is computed before start (teacher GPU rate × estimated hours from dataset token count) and shown inline. No surprise bills.
- Ineligible cases show nothing — no greyed-out options to confuse people ("why can't I click this?").

### Hosted-teacher picker addition

The Stage 1 "Recommended open models" radio-card gains hosted entries (from the catalog) with size/GPU labels ("Qwen 32B · runs on 1 GPU, billed per minute of use"). Choosing one = provider policy `allowed`, no acknowledgment friction.

### Advanced options additions (same single accordion)

| Knob | Default | Notes |
| --- | --- | --- |
| Fidelity detail (`top_k`) | 128 | helper: "How much of the teacher's confidence to keep. Default is right for almost everyone." |
| Loss mix (`kd_alpha` / `ce_alpha`) | 0.9 / 0.1 | expert-only wording, collapsed sub-group |
| Teacher precision | fp8 | fp8/int4/bf16; affects cost estimate live |

### Progress + failure UX

- Pipeline view gains one stage chip when fidelity is on: **"Scoring with teacher"** between Generate and Train, with the same progress/heartbeat treatment as training.
- Tokenizer mismatch (race: catalog changed between check and run): job fails fast with *"These two models read text differently, so high-fidelity training isn't possible between them. Standard distillation works — switch and re-run."*
- Spend cap hit: *"This run reached your GPU spending cap for teachers. Raise the cap in Settings → Billing or resume with a smaller dataset."*

## 2. Engineering tasks

Order matters; each task lands green independently. Same commit rules as Stage 1.

### Task 1 — Hosted teacher config + catalog (Rust)

**[after-S1]** final field shapes align to Stage 1's merged `TeacherConfigDto`.

- Extend teacher DTO with `kind: "endpoint" | "hosted"`; hosted carries `model_id`, `precision` (`fp8` default), `top_k_logprobs` (128), derived GPU spec. **Fidelity selection is `distill_method: "text" | "logit"` inside the teacher/distill config — NOT the existing `TrainingMethod` enum (`qlora|lora|full`), which stays untouched; they are orthogonal concepts.**
- Catalog: extend the existing model-catalog source with `teacher: { gpu: "...", est_tok_per_sec: n }` entries for 2–3 launch teachers (same-family pairs with our student catalog — e.g., one Qwen mid-size, one Llama mid-size).
- **Hosted-teacher supply-chain rules (security boundary):** teacher `model_id` must be a catalog entry — tenant-supplied arbitrary model ids are rejected; catalog entries carry a **pinned revision/commit** and license metadata; model loading always `trust_remote_code=False` unless a catalog entry is explicitly reviewed and flagged; model weight caches are shared-read platform artifacts, never tenant-writable.
- **Token-count preflight:** before estimating or admitting an extraction job, compute and store per-dataset `prompt_tokens`, `completion_tokens`, `scored_completion_tokens` (**shared-tokenizer** counts — Stage 2 requires exact teacher/student tokenizer identity, so there is exactly one tokenizer; cheap pass over the dataset). Stage 1 stores pair counts only — these fields are new and the estimate depends on them.
- Cost-estimate endpoint: `POST .../teacher-cost-estimate { dataset_id, teacher }` → `{ est_gpu_hours, est_cost }` from the preflight token counts × catalog throughput × provider rate. Pure function + unit tests.
- Policy: hosted catalog entries classify `Allowed`.

### Task 2 — Tokenizer identity guard (workers)

- New pure module `tokenizer_identity.py`: download tokenizer artifacts for two model ids (tokenizer.json / vocab+merges, `special_tokens_map`, `added_tokens`, chat template), canonicalize, SHA-256 each, compare **all** — any mismatch = incompatible. Cache hashes per model id.
- Exposed two ways: an API-callable preflight (drives the UX recommendation card) and a hard re-check as the first step of the extraction workflow (fail fast, non-retryable, UX message above).
- Tests: same-model → match; same family/different special tokens → mismatch; fixture-based (no network in tests).

### Task 3 — Teacher logprob extraction job (workers + Modal)

The one genuinely new GPU workload. **Self-contained batch job** (deliberately NOT a server — no network coordination, matches scale-to-zero):

- New Modal function (pattern-match the existing training function): load teacher via **vLLM offline `LLM` class**, `SamplingParams(prompt_logprobs=k, max_tokens=1)`; for each record feed the **rendered prompt+completion** as the prompt and extract logprobs at completion-token positions only.
- **Token-boundary bookkeeping (mandatory — one BOS/template difference silently shifts every target):** rendering uses the shared teacher/student tokenizer + chat template; per record, store `prompt_token_count`, `completion_start`, `completion_len`, and the label mask; the manifest stores the **rendered-template hash** alongside the tokenizer hash. Training refuses artifacts whose template hash doesn't match its own rendering — same defense-in-depth as the tokenizer check.
- vLLM specifics (verified): server/engine `max_logprobs` defaults to 20 — set `--max-logprobs k` (never `-1`/uncapped: OOM); budget context 8–16K; tune `gpu_memory_utilization` up for batch scoring.
- **Pre-implementation spike (half-day, required):** verify vLLM offline `prompt_logprobs` behavior on a rendered prompt+completion before writing the extraction job — specifically: position 0 returns no logprob (no context) and how that shifts indexing; whether `max_tokens=1` (one generated-and-discarded token) has side effects; that the actual token's logprob is present alongside the top-k; and BOS handling under our chat template. The boundary bookkeeping above is designed to absorb these — the spike confirms the exact offsets.
- **Determinism caveat (recorded, accepted):** vLLM prompt-logprobs can differ slightly across batch sizes (numeric, kernel-dependent — known upstream issue). Harmless for KD soft targets; record `vllm_version` + batch config in the artifact manifest so runs are explainable.
- Checkpoint per shard (resume, don't restart); heartbeat; GPU-queue concurrency respected; billing rows per Task 5.
- **Observability (wire into the existing OTel/Prometheus/Grafana stack, not new plumbing):** structured logs + spans for the extraction job; metrics: extraction duration, scored tokens/sec, shard progress, retries, teacher load time. A "distillation" row/panel on the existing training dashboard — extraction and training runs visible side by side.

### Task 4 — Logprob artifact format (workers)

Per spec §6.2, concretely:

- Shard = one file per N records (N sized so shards ≈ 64–128 MB): columnar arrays `token_ids: uint32[tokens][k]`, `logprobs: float16[tokens][k]`, `tail_mass: float16[tokens]`, **`support_len: uint16[tokens]`** (how many of the k entries are real — vLLM can return fewer than k, and some positions return none), plus per-record offsets and the Task 3 boundary fields (`completion_start`, `completion_len`, label mask).
- **Padding rule (loss-correctness):** entries beyond `support_len` are padding and MUST be excluded from the KL — the loss gathers only real support; a unit test feeds a shard with short rows and asserts padding contributes zero mass.
- **Tail-mass math (specified):** `tail_mass = max(0, 1 - sum(exp(logprobs[:support_len])))` — summed over real entries only, computed in **fp32**, stored fp16. Tests: numeric clamping (fp noise pushing the sum above 1 → exactly 0), rows with `support_len < k`, and a hand-computed reference case.
- Container: **plain `np.savez_compressed` (.npz)** — numpy read/write, zero new deps, whole-shard sequential loads (shards are sized for it; no random access requirement). Revisit a columnar format only if streaming profiling demands it.
- `manifest.json` per dataset: shard list → record ranges, `k`, teacher id + pinned revision, tokenizer hash, **rendered-template hash**, vllm version, created_at.
- Location: under the tenant's dataset S3 prefix (inherits RLS-scoped access, dataset deletion, tenant erasure — Stage 1 §9 table holds).
- Retention: deleted on training-job completion + configurable grace (`TEACHER_ARTIFACT_RETENTION_HOURS`, default 72) via a cleanup activity — derived data, regenerable.

### Task 5 — Teacher-GPU billing + per-tenant caps (Rust + workers)

- Extraction is a **first-class billable workload, not "like training" by analogy** — the existing reservation plumbing is keyed to `training_jobs`/`evaluations`, so extraction gets its own explicit state: `training_jobs.teacher_extraction_{status, modal_call_id, cost}` columns (migration) plus billing rows with a distinct extraction kind. Reservation pattern applies verbatim: conservative pending row before the GPU starts → finalize with actuals → stale reaper covers crashes/cancellations. No fire-and-forget, and a cancelled extraction is distinguishable from a cancelled training run.
- New config `TEACHER_GPU_SPEND_CAP_*` (per-tenant, monthly): checked at job admission (hard 402-style refusal with the UX message) and at shard checkpoints (graceful stop + partial-artifact cleanup).
- Extraction jobs count toward `GPU_MAX_CONCURRENT_ACTIVITIES`.

### Task 6 — `distill_logit` strategy (workers)

The new ML surface. **Dispatch is explicit, not implied:** the public `TrainingMode` stays `distill` (never `"distill_logit"`); at strategy lookup, the activity resolves the registry key from `mode` + `distill_method` (`distill` + `text` → the Stage 1 SFT strategy; `distill` + `logit` → `distill_logit`). A flat composite-key mapping at the dispatch site — one function, unit-tested — rather than a delegating wrapper strategy: fewer layers, same guarantee that no internal strategy name ever leaks into the public enum or API.

- Custom compute-loss on top of the existing engine/LoRA student:
  `loss = kd_alpha · KL_fwd(teacher_topk+tail ‖ student) + ce_alpha · CE(labels)`, defaults `kd_alpha=0.9, ce_alpha=0.1, T=1.0` (forward KL — off-policy teacher-forced, per research §4).
  KL over the top-k support: gather student logprobs at teacher token ids; tail bucket treated as one merged pseudo-token (reduces truncation bias; documented as not equivalent to full-vocab KD).
- Streams shards; asserts manifest tokenizer-hash == student tokenizer-hash at load (defense in depth).
- **Tests are the point:** pure-loss unit tests on synthetic tensors (hand-computed KL cases, tail handling, alpha mixing, masking of prompt positions); a tiny smoke config **[after-S1]** validated on Modal like the existing training smoke test.

### Task 7 — Fidelity metric in parity suite (workers)

- When artifacts exist for the eval split, `TeacherParitySuite` adds **`teacher_student_kl` = mean per-token forward KL(teacher ‖ student)** over teacher support + tail — the only direction computable from teacher-top-k artifacts (reverse KL would need teacher logprobs on the *student's* chosen tokens, which we don't store), and it matches the Stage 2 training objective. UI shows it in the parity panel as "distribution match" with a plain tooltip. Report-only.

### Task 8 — UI (web)

- Build §1 exactly: recommendation card + inline cost estimate (from Task 1 endpoint), hosted entries in the teacher picker, accordion additions, "Scoring with teacher" pipeline chip, the two failure messages.
- Gates: `pnpm type-check`, `pnpm lint`.

### Task 9 — Docs, gates, PR

- `docs/DEVELOPMENT.md`: `TEACHER_GPU_SPEND_CAP_*`, `TEACHER_ARTIFACT_RETENTION_HOURS` rows.
- DESIGN-SPEC status → "Stage 2 implemented".
- Full workspace gates (same list as Stage 1 Task 8); PR `Add high-fidelity distillation with hosted teachers`; **user merges**.

## 3. Explicitly deferred to Stage 3

On-policy loop, teacher-server topology and the TRL `loss_top_k` decision, reverse-KL training, calibration (ECE) metrics.

## 4. End-to-end verification (manual, before merge)

1. Eligible pair (catalog Qwen teacher + Qwen student): recommendation card appears with a sane cost estimate; ineligible pair shows nothing.
2. Fidelity run on a small dataset: extraction job produces shards + manifest; kill it mid-run → resumes from checkpoint; billing rows reserve→finalize correctly.
3. Training consumes artifacts; loss decreases; adapter exports; parity report includes `teacher_student_kl`.
4. Tokenizer-mismatch forced (edited catalog entry) → fast, human-readable failure. Spend cap set to ~0 → admission refused with the cap message.
5. Artifacts deleted after retention window; dataset deletion / tenant erasure removes them immediately.

## Key sources

- vLLM engine args / logprobs (`max_logprobs`, `prompt_logprobs`): <https://docs.vllm.ai/en/stable/configuration/engine_args/> · <https://docs.vllm.ai/en/latest/api/vllm/logprobs/>
- vLLM prompt-logprobs batch-variance issue (determinism caveat): <https://github.com/vllm-project/vllm/issues/11778>
- vLLM optimization/tuning (gpu_memory_utilization): <https://docs.vllm.ai/en/latest/configuration/optimization/>
- Offline top-k KD reference design (decoupled extraction → train): axolotl KD plugin — <https://docs.axolotl.ai/docs/api/integrations.kd.trainer.html>
- Loss/tail reference: TRL distillation trainer (`loss_top_k`, `loss_add_tail`) — <https://huggingface.co/docs/trl/distillation_trainer>
- Full evidence base: [RESEARCH.md](RESEARCH.md) §8 (libraries, hyperparameters, hosting costs)
