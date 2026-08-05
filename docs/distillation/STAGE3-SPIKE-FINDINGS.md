# Stage 3 spike findings — what the libraries actually do

> Companion to [IMPLEMENTATION-PLAN-STAGE3.md](IMPLEMENTATION-PLAN-STAGE3.md). Everything here was read out of TRL's shipped source at tag **v1.9.2** (released 2026-07-28) or measured, not taken from documentation. Where TRL's docs and TRL's code disagree, the code wins and the disagreement is recorded.

## 1. TRL's on-policy distillation documentation describes an API that does not exist

`docs/source/distillation_trainer.md` (v1.9.2) tells you to write:

```python
config = DistillationConfig(
    use_teacher_server=True,
    teacher_model_server_url="http://teacher-host:8000",
    loss_top_k=1,
    beta=1.0,
    lmbda=1.0,
)
```

**This raises `TypeError`.** `DistillationConfig` in the same release has no `use_teacher_server`, no `teacher_model_server_url`, no `loss_top_k`, and no `lmbda`. The plan's Option A was written from this page, which is why it named a configuration that cannot be constructed.

Verified by field-listing the shipped dataclasses rather than trusting either the page or a search result. Two independent web searches also asserted the documented API exists — including specific claims about binary logprob payloads and `use_teacher_server` — so the failure mode here is not "one stale page", it is that everything except the source agreed on an API that isn't there.

## 2. The three real modules

| Module | Teacher | Loss support | `lmbda` (on-policy fraction) |
| --- | --- | --- | --- |
| `trl.experimental.distillation` | **local only** (in-process) | full vocab, generalized JSD | no — always on-policy |
| `trl.experimental.server_distillation` | **server** (`teacher_model_server_url`) | sparse: `beta>0` ⇒ `loss_top_k == 1` | no |
| `trl.experimental.iw_opd` | **local or server** (`use_teacher_server`) | same sparse rule when server-backed | **yes** |

`iw_opd` (Importance-Weighted On-Policy Distillation) is a strict superset of the other two in configurability: `lmbda`, `beta`, local-or-server teacher, and `distillation_objective ∈ {"jsd", "iw_opd"}` — `"jsd"` being the same generalized JSD/KL objective the other modules compute.

All three live under `trl.experimental`, whose contract is explicit: *"APIs may change without notice while the feature is iterated on."* See §6 for what that forces on us.

## 3. `GKDTrainer` no longer exists

`trl/trainer/` at v1.9.2 contains no `gkd_trainer.py`. The on-policy trainer our own `RESEARCH.md` cited as "TRL's `GKDTrainer`" was removed and replaced by the modules in §2. `RESEARCH.md` has been corrected.

This is direct evidence for §6: TRL removes public trainers between minor versions.

## 4. A server teacher is not an OpenAI-compatible endpoint

`ServerDistillationTrainer` and `iw_opd` both fetch teacher logprobs through TRL's own `VLLMClient`, which POSTs to **`/get_sequence_logprobs/`** — an endpoint that exists only in TRL's `trl vllm-serve`, not in `vllm serve`. Consequences:

- The teacher process must be `trl vllm-serve --model … --revision … --port …`, so the teacher image needs **both** `trl` and `vllm`.
- `get_sequence_logprobs` scores *existing* token sequences given `prompt_lengths` — the same operation Stage 2 does offline with `prompt_logprobs`, exposed over HTTP.
- `VLLMClient.__init__` calls `is_vllm_available()` and raises without it, so **vLLM must be installed in the trainer container too**, purely to construct an HTTP client. This is what forces §5.
- The constructor health-checks `GET /health/` and blocks until it answers or `connection_timeout` elapses; the trainers pass 60 s.
- The client sends no authentication of any kind.

## 5. The trainer cannot be our existing training image

Our training image is Unsloth-based, and `modal_app.py` already documents why vLLM cannot join it: *"unsloth pins its own torch build and resolving both together produces a broken pair."* Since §4 requires vLLM inside the trainer, the on-policy trainer needs a **separate image without Unsloth** — plain `transformers` + `peft` + `trl` + `vllm`.

That is why Stage 3 adds a second `TrainingEngine` implementation (`transformers`) behind the existing `register_engine` seam rather than modifying `UnslothEngine`. The abstraction was already there; this is its first second implementation.

## 6. Version pins: two different vLLMs, and TRL must stop floating

TRL v1.9.2 requires **`vllm>=0.17.0,<=0.25.1`**. Our Stage 2 extraction image pins `vllm==0.26.0`, which is *above* that ceiling. So:

- The Stage 2 offline extraction image keeps `vllm==0.26.0` — its measured `prompt_logprobs` contract belongs to that version.
- The Stage 3 on-policy image pins `vllm==0.25.1` — the newest version TRL supports.

Both pins are deliberate and independent; neither may be bumped without re-measuring the thing that depends on it.

Separately, `trl>=0.16.0` (unpinned upper bound, in both `pyproject.toml` and the Modal images) was a latent break: floating from 0.16 to 1.9 removed `GKDTrainer` (§3) and moved the whole distillation surface into `trl.experimental`. Depending on an explicitly unstable namespace with an open-ended version range means a rebuild can change training behaviour with no code change. **TRL is now pinned exactly.**

## 7. Loss shape, and a Stage 2 correction

Server-backed distillation is constrained by what a top-k-only teacher can answer, and `iw_opd`/`server_distillation` enforce it in `__post_init__`:

- `beta == 0` (forward KL) → `loss_top_k` must be ≥ 1, computed over the teacher's top-k.
- `beta > 0` (reverse KL or JSD) → `loss_top_k` must be **exactly 1**, and the top-1 token must be the **sampled** completion token.
- `reverse_kl_top_1_mode="argmax"` is rejected with a server teacher, because the server *cannot score arbitrary student-selected tokens*.
- `loss_add_tail` (default `True`) appends a tail bucket for the probability mass outside the selected support.
- `use_liger_kernel` is rejected on every server path.

The `argmax` restriction is principled, not an oversight: reverse KL wants teacher probabilities on the student's high-probability tokens, and a top-k endpoint only returns the teacher's own ranking. A hand-rolled top-k reverse KL would hit exactly the same wall, so there is no richer signal available to us for the same money.

**Correction to [STAGE2-SPIKE-FINDINGS.md](STAGE2-SPIKE-FINDINGS.md):** that document states TRL's `loss_top_k`/`loss_add_tail` "DO NOT EXIST". They do exist — in `server_distillation` and `iw_opd`, which are not where Stage 2 looked. Stage 2's *decision* to write its own offline loss remains correct for a different reason than the one recorded: both of those trainers require a **live teacher server**, so neither can consume precomputed artifacts, which is the entire premise of Stage 2. The finding has been rewritten to say that.

Also worth recording: TRL's `loss_add_tail` models the out-of-support mass as an explicit tail bucket rather than renormalizing it away. Stage 2 chose the same shape independently, over the renormalizing reference implementation it was compared against. That is corroboration, not coincidence.

## 8. Batch-size invariant worth validating early

`iw_opd.__post_init__` enforces:

```
generation_batch_size * num_generations == per_device_train_batch_size * gradient_accumulation_steps
```

and derives `generation_batch_size` when it is unset. A violation raises inside the trainer — i.e. after a GPU has been paid for and a teacher has loaded tens of gigabytes of weights. Stage 3 therefore checks it in the API, before admission, the same way Stage 2 checks tokenizer identity up front.

`distillation_objective="iw_opd"` adds its own constraints: `lmbda` must be 1.0, `reverse_kl_top_1_mode` must be `"sampled"`, and with `use_vllm=True` it requires `vllm_sync_frequency=1`.

## 9. What was NOT measured

No GPU was spent on this spike. Every claim above is a source-verified API fact or an arithmetic consequence, which is why the topology decision in the plan's §2 could be settled without the 2–3 day A/B run it originally called for (see that section for the reasoning).

What remains genuinely unmeasured, and is therefore called out in [STAGE3-TESTING.md](STAGE3-TESTING.md) rather than assumed here:

- whether top-1 sampled reverse KL improves parity over the Stage 2 off-policy baseline at all, and by how much;
- rollout throughput with `use_vllm=False` (the shipped default) versus colocated vLLM generation;
- whether a 32B teacher and an 8B student actually co-schedule on one 2×80 GB container in practice;
- the real wall-clock cost of an improve pass, which the cost estimate currently approximates.
