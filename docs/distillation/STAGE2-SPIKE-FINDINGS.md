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
