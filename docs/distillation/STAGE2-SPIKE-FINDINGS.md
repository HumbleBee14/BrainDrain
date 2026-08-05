# Stage 2 spike — measured vLLM `prompt_logprobs` behavior

> Required pre-implementation spike from [IMPLEMENTATION-PLAN-STAGE2.md](IMPLEMENTATION-PLAN-STAGE2.md)
> Task 3. Run 2026-08-04 on Modal (A10, `Qwen/Qwen2.5-0.5B-Instruct`, `top_k=8`),
> **vLLM 0.26.0**. These are measurements, not assumptions — the extraction
> job's index bookkeeping and the artifact format depend on them.

## Findings

**1. `prompt_logprobs` aligns 1:1 with `prompt_token_ids`.** 40 tokens in, 40
entries out. Entry `i` is the distribution over the token *at* position `i`
given everything before it, so the distributions supervising a completion are
`plp[completion_start : completion_start + completion_len]`.

**2. Exactly one entry is `None`: position 0.** Nothing precedes the first
token, so there is no distribution for it. No other position was ever `None`.

**3. `max_tokens` does not perturb the prompt logprobs.** `max_tokens=1` and
`max_tokens=8` produced byte-identical logprobs, so scoring uses `max_tokens=1`
and discards the generated token.

**4. Support size is `k` OR `k+1`, not "at most k".** With `top_k=8` requested,
observed support lengths were **8 and 9**. vLLM returns the top-k *plus the
actual token* when the actual token falls outside the top-k — and the actual
token's logprob was present at every position.

> **This contradicts the plan**, which anticipated only "vLLM can return fewer
> than k". Artifact arrays must therefore be width **k+1**, and `support_len`
> must be trusted as the real bound in both directions. A width-k array would
> silently drop the actual token's probability at exactly the positions where
> the teacher was surprised — the most informative positions in the dataset.

**5. Rendered prompt ids are a clean prefix of prompt+completion ids.** vLLM
served exactly the tokenizer's ids with nothing prepended (`served_equals_full_ids`
= true; Qwen2.5 has no BOS token), so `completion_start = len(tokenize(rendered_prompt))`
is correct. This must stay a *measured* property per model family, not an
assumption — hence the rendered-template hash in the manifest.

**6. Batch size did not change values at this scale.** A batch of 2 matched the
single-request logprobs exactly. The known upstream batch-variance issue is
still recorded in the manifest (`vllm_version` + batch config) rather than
assumed absent at production batch sizes.

## Tokenizer pairing — measured, and it changed the catalog

Compared with the Task 2 guard (tokenizer/vocab, `special_tokens_map`,
`added_tokens`, chat template — canonicalized, then SHA-256'd):

| Pair | Verdict |
| --- | --- |
| `Qwen/Qwen2.5-32B-Instruct` ↔ `Qwen/Qwen2.5-7B-Instruct` | **compatible** — identical combined hash |
| `Qwen/Qwen2.5-32B-Instruct` ↔ `unsloth/Qwen2.5-7B-Instruct` | **incompatible** — `vocab`, `special_tokens_map`, `added_tokens` all differ |

The community re-upload is not a cosmetic difference: upstream Qwen ships **no
`special_tokens_map.json` at all**, while the re-upload adds one declaring
`pad_token: "<|vision_pad|>"` plus an `additional_special_tokens` list that
exists nowhere upstream. It also serializes `tokenizer.json` merges in the newer
list-of-pairs form and adds an `ignore_merges` key.

Our base-model catalog contained *only* the re-upload, so before this change
logit distillation could never have been eligible for any student we offer. The
catalog now also carries upstream **`Qwen/Qwen2.5-7B-Instruct`** as the
distillable student, and hosted entries list upstream repositories only.

**Accepted tradeoff:** the guard is conservative. Two repos with genuinely
equivalent vocabularies can still fail it because different `tokenizers` library
versions serialized them differently. That is the correct direction to fail —
a silent tokenizer mismatch corrupts every training target — but it means a
rejection is not proof the models are incompatible, only that we cannot prove
they are compatible. The user-facing copy says so without jargon.

## Literature and library check (2026-08-04) — four defaults changed

Verified against current sources rather than the plan's assumptions. Each item
below **changed a default or a design choice**.

**No library implements our workflow — write the loss.** Every TRL distillation
trainer needs the teacher *live* — either in memory or behind an HTTP endpoint it
queries during training — and none has a field for precomputed teacher logprobs.
That, not the parameter set, is what rules TRL out for an offline artifact path.

> **Corrected 2026-08-04 (Stage 3).** This finding originally claimed the plan's
> cited `loss_top_k` / `loss_add_tail` parameters "do not exist in TRL's source".
> They do exist — in `trl.experimental.server_distillation` and
> `trl.experimental.iw_opd`, neither of which Stage 2 examined. The conclusion
> below is unchanged, because both of those trainers require a live teacher
> server and so cannot read Stage 2's artifacts; only the stated reason was
> wrong. TRL's `loss_add_tail` also models the out-of-support mass as an explicit
> tail bucket, independently corroborating the tail term chosen below over
> Axolotl's renormalization. See
> [STAGE3-SPIKE-FINDINGS.md](STAGE3-SPIKE-FINDINGS.md) §7.

Axolotl's KD plugin *is* real and genuinely offline (reads top-k ids + logprobs
from a dataset field, `kd_alpha`/`kd_ce_alpha`/`kd_temperature` defaulting to
exactly 0.9 / 0.1 / 1.0), so it is the structural reference to follow — but its
loss renormalizes over the top-k and therefore **discards the tail** rather than
modelling it. NeMo-Aligner supports offline top-k KD at K≈100 but drags in the
whole Megatron stack. Conclusion: custom `compute_loss`, shaped like Axolotl's
chunked forward-KL, plus our own tail term.

**Top-k default 128 → 32.** Published results put the useful range at k≈5–10
with sharp diminishing returns (Zipfian token distributions), and vLLM's cost
grows on three axes with k at once — artifact bytes, scoring memory, and a
roughly linear slowdown in the requested logprob count (vLLM issue #14300). 32
keeps a wide margin over the reported optimum at a quarter of the planned cost.

**Teacher precision default fp8 → bf16.** fp8 is documented to cause real
accuracy regressions in some regimes and *no* published measurement exists for
what it does to logprobs specifically. The entire product of this pass is the
teacher's probability distribution, so quantizing the teacher to save GPU time is
the one saving that could invalidate the feature. fp8 stays available as a knob.

**Teacher family → Qwen3.** Every dense Qwen3 size shares a byte-identical
tokenizer (verified with our own guard: `Qwen3-32B` ↔ `Qwen3-8B`/`4B`/`14B` all
produce combined hash `6310389b…`), Apache-2.0, ungated, and dense. Qwen2.5 and
Qwen3 are **not** cross-compatible (vocab 152064 vs 151936), so they are separate
catalog entries with separate student lists, never mixed. Mixture-of-experts
teachers are excluded: they add routing nondeterminism to a pass whose product is
reproducible probabilities, and no measurement was found either way.

**Forward KL confirmed correct** for this off-policy/teacher-forced setting
(reverse KL belongs to on-policy student rollouts, which is Stage 3). The field
is trending toward on-policy distillation as stronger, which is precisely why
Stage 3 exists — offline KD stays worthwhile because one teacher pass is reusable
across many student runs.

**Tail handling, and what we deliberately do not claim.** Recent work
(arXiv:2602.20816) reports that a *separate renormalized tail term with an
amplification factor* (β≈2) beats both discarding the tail and lumping it into
one flat pseudo-token. We implement the decoupled form so the mechanism exists,
but default `tail_beta = 1.0` — the unamplified case. Shipping someone else's
tuned constant as our default would be asserting a result we have not measured on
our own data; the parity harness is what would earn that change.

**Batch composition is part of the determinism contract.** vLLM issue #11778
reports prompt-logprob differences between batch sizes larger than float noise,
closed as not planned. Our own measurement at small scale showed no difference,
which does not clear it at production batch sizes — so the manifest records the
batch configuration, and identical logprobs are never assumed across differently
batched runs of the same data.

**No cheaper bulk-scoring API exists.** vLLM's pooling/scoring API is for
cross-encoder similarity, not token-level logprobs. `generate()` with
`prompt_logprobs` and `max_tokens=1` remains the only path.

## Boundary verified against the real tokenizer

`render_record` on `Qwen/Qwen3-8B` with a three-turn conversation: 36 tokens
total, completion starting at 24, and the supervised span decoding to
`'<think>\n\n</think>\n\nBlue is a primary colour.<|im_end|>\n'`. The
rendering fingerprint is identical for `Qwen3-8B` and `Qwen3-32B`, which is the
whole premise of the feature holding end to end.

Note the empty `<think></think>` block: Qwen3's chat template injects it into
every assistant turn. It is supervised along with the answer, which is correct —
both the teacher's scoring pass and the student's training pass render through the
same template, so the two agree, and the student learns Qwen3's actual
non-thinking output format rather than a format it will never be asked to
produce. It does mean a few scored tokens per record carry no information; the
alternative (stripping them) would introduce exactly the train/serve skew the
fingerprint exists to prevent.

## Operational finding

vLLM's FlashInfer sampler JIT-compiles a CUDA kernel at engine warmup and fails
with `AssertionError` in `_find_cuda_home` on an image without the CUDA toolkit
(`modal.Image.debian_slim` + `pip install vllm`). Scoring never samples, so the
extraction image sets **`VLLM_USE_FLASHINFER_SAMPLER=0`** rather than carrying
`nvcc` — smaller image, one less build dependency. This cost one wasted GPU
start to discover.

## Reproducing

The probe script is throwaway (not committed). It renders a chat prompt +
completion with the shared tokenizer, calls
`LLM(..., max_logprobs=k).generate([full_text], SamplingParams(temperature=0.0,
max_tokens=1, prompt_logprobs=k))`, and reports alignment, `None` positions,
support sizes, whether the actual token is in support, and the prompt/completion
offsets.
