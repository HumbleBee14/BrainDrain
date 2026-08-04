# Distillation Stage 3 — Implementation Plan (on-policy)

> Implements [DESIGN-SPEC.md](DESIGN-SPEC.md) §7 on top of Stages 1–2.
> Status: **decision-gated.** The topology choice (§2) can only be made honestly after Stage 2 runs on real jobs — this plan fixes the architecture, the product surface, and the decision procedure, and marks execution details **[after-S2]**.
> Branch: `feat/distillation-stage3`. PRs only — the user merges.

---

## 0. What Stage 3 adds

Stages 1–2 train the student on **teacher-written** data (off-policy). Stage 3 closes the loop: the **student generates, the teacher grades the student's own tokens**, fixing exposure bias — the failure mode where a student only ever trained on perfect teacher text falls apart on its own mistakes. This is the phase-2 recipe the strongest small open models use, at ~1/10 the cost of RL (evidence: RESEARCH.md §3.3, §8.3).

New surface: **two coordinated GPU workloads** (teacher server + student trainer) — the genuinely new infrastructure of the whole distillation track.

## 1. Product surface: "Improve this model" (zero new decisions at setup)

On-policy is **not** a third option at training setup — that would break the three-decision rule for a subtle expert concept. Instead it is an **improvement pass on an already-distilled model**:

- Model page of any distill-mode model (with a hosted, tokenizer-compatible teacher recorded in provenance) shows one card:
  > **Sharpen against the teacher**
  > Your model retrains on its *own* answers, corrected token-by-token by the teacher. Best when answers drift over long outputs or multi-step tool use. Estimated **~$X.XX** GPU time.
  > [Improve]
- One click → new training job (`distill` mode, `distill_method: "on_policy"` in the distill config — the public `TrainingMethod` enum `qlora|lora|full` stays untouched, same rule as Stage 2), same provenance chain, parity report re-runs → the model page shows **before → after parity** ("matched 87% → 94%").
- **Eligibility requires complete hosted-teacher provenance**, not just "a teacher was recorded": pinned model revision, tokenizer + rendered-template hashes, GPU spec, precision, and provider policy — everything needed to boot the *same* teacher again. Endpoint-only teachers, tokenizer mismatch, or incomplete provenance → the card doesn't render.
- Advanced accordion (only knobs): rollout sampling temperature (default 1.0), on-policy fraction λ (default 1.0), steps budget.

This also matches the research recipe exactly (off-policy phase → on-policy phase), so the product flow *is* the correct ML flow.

## 2. The topology decision (the real engineering choice)

| Option | How | Pros | Cons |
| --- | --- | --- | --- |
| **A. TRL server-mode, top-1 sampled reverse KL** | Teacher on separate vLLM server; TRL `DistillationTrainer` with `loss_top_k=1`, `beta=1.0` | Supported path in a maintained library; teacher size unlimited; least custom code | Top-1 sampled signal (documented TRL constraint for `beta>0` remote); per-token signal is sparser |
| **B. Colocated teacher, full-vocab reverse KL** | Teacher loaded in the trainer process | Richest signal (full distribution) | Teacher must fit next to student on training GPUs → caps teacher ≈13B fp8 on 1×H100-class; kills the big-teacher story |
| **C. Custom trainer** | Own loop: student rollouts (vLLM) → teacher top-k scoring → custom reverse-KL-over-top-k loss with bias correction | Exact loss we want; top-k reverse KL with correction is research-supported | Most code to own; correctness burden on us |

**Decision procedure [after-S2]:** a 2–3 day spike distills the same small pair (e.g. Qwen mid → Qwen small on a tool-call dataset) via A and B, comparing parity gain per GPU-dollar. Ship the winner. **C's threshold is defined up front, not by vibes:** build the custom trainer only if BOTH A and B either (a) fail to improve parity by ≥1 point over the Stage 2 off-policy baseline on the spike pair, or (b) are unstable (divergence/NaN/collapse on ≥2 of 3 seeds). Default expectation going in: **A** (managed constraint, real on-policy, big teachers), with B as the small-teacher fast path.

## 3. Infrastructure: coordinated teacher + trainer (the new hard part)

- **Teacher server job:** Modal vLLM server app (OpenAI-compatible + logprobs), started per training job, private networking to the trainer, torn down when the trainer exits **or on a TTL reaper — a leaked teacher GPU is a silent money fire; teardown is enforced by both the workflow finally-path and an independent stale-server reaper** (reservation-pattern thinking applied to serving).
- **First-class lifecycle state (auditable, not implicit):** `teacher_server_{status, modal_call_id, started_at, expires_at, cost}` persisted on the training job (migration) — the reaper works off `expires_at`, billing finalizes off actuals, and "was a teacher leaked?" is answerable with a query, not log archaeology.
- **Observability:** teacher server registers on the existing OTel/Prometheus stack — uptime, requests/sec from the trainer, health-check failures, and a **reap-event counter with alert** (a fired reaper means the normal teardown path failed — that's a bug signal, not just a cost save).
- **Trainer job:** existing training container + TRL trainer pointed at the teacher URL; retries/backoff on transient teacher unavailability; **fails the job (non-retryable) if the teacher can't come back** — never trains silently without supervision signal.
- **Ordering:** workflow starts teacher → health-checks → starts trainer → on any terminal state tears down teacher. Both workloads emit reservation-pattern billing rows; both count toward GPU concurrency and the Stage 2 spend caps.
- **Failure matrix (enumerate in tests):** teacher OOM at boot; teacher dies mid-training (trainer checkpoint-resumes after teacher restart, bounded retries); trainer dies (teacher reaped by TTL); workflow worker dies (Temporal retries; teacher reaper is the backstop).

## 4. Evaluation additions

- Parity report gains **before/after** comparison against the parent model's evaluation (same golden set — comparable by construction).
- **Calibration (ECE) — design spike required before implementation.** "ECE" for generative models is ambiguous (token-level confidence? answer-correctness confidence? judge-calibrated?). A small spike picks the definition and validates it on one model pair; only then does it enter `TeacherParitySuite`. Report-only, never gated, regardless of definition.
- **Reverse-KL fidelity metric — scoped to what the topology can compute:** reverse KL(student‖teacher) needs the teacher to score *student-generated* tokens, which Stage 2 artifacts cannot provide. So: the per-step training reverse-KL trend is logged for free (it *is* the training signal); a fresh holdout reverse-KL is computed **only while the teacher server is still up** (eval phase ordered before teardown). If the chosen topology makes that awkward, report the training-trend endpoint value and label it as such — never a fabricated "holdout" number.

## 5. Engineering tasks (skeleton — firmed [after-S2])

1. **T0 spike:** A-vs-B benchmark on one pair; write the decision into this doc.
2. **T1 teacher-server lifecycle:** Modal server app, health checks, TTL reaper, teardown paths, billing rows. (Most of the risk lives here.)
3. **T2 trainer integration:** chosen topology wired into a `distill` on-policy strategy (`distill_method: "on_policy"`, resolved through the same composite-key dispatch as Stage 2); rollout buffering; checkpoint/resume story with a live teacher dependency.
4. **T3 API/UI:** "Improve this model" card + eligibility + cost estimate + before/after parity view.
5. **T4 eval:** ECE + before/after wiring.
6. **T5 docs/gates/PR** — same rules as prior stages.

## 6. Out of scope

Cross-tokenizer on-policy (GOLD) until stable upstream; multi-teacher (MOPD); step-level reweighting for agents (SOD) — candidate Stage 4 research track; RL-style reward mixing.

## 7. Verification (manual, before merge)

1. Improve-pass on a Stage 2 model: teacher boots, trains, tears down (verify zero orphaned teacher servers after: success, trainer crash, workflow kill).
2. Before/after parity visibly reported; billing rows for both workloads reserve→finalize; spend cap stops admission.
3. Teacher killed mid-run → trainer resumes or fails loudly; never silent low-quality completion.
