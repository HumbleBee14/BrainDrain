"""On-disk format for a teacher's precomputed token distributions.

One shard holds the scored positions of many records in flat columnar arrays,
addressed CSR-style by per-record offsets, so a training step reads a whole shard
sequentially and slices it without parsing.

Two decisions here are load-bearing and both come from measurement rather than
assumption (see docs/distillation/STAGE2-SPIKE-FINDINGS.md):

**Width is `k + 1`, not `k`.** vLLM returns the top-k *plus the actual token*
when the actual token falls outside the top-k, so support size per position is
`k` or `k+1`. A width-`k` array would drop the actual token's probability at
exactly the positions where the teacher was surprised — the most informative
positions in the data.

**The exact token ids the teacher scored are stored.** Training could re-render
the dataset instead, but then a tokenizer or template difference would silently
shift every target. Four bytes per token is nothing next to `(k+1) x 6`, and it
makes the class of bug impossible rather than merely unlikely.

Padding beyond `support_len` is present in the arrays and MUST be excluded from
any loss; `support_len` is the only authority on how many entries are real.
"""

import json
from dataclasses import dataclass, field

import numpy as np

# Padding token id for entries beyond a position's real support. Never read —
# `support_len` bounds every gather — but a defined value keeps shards
# byte-reproducible and makes accidental reads obvious rather than random.
PAD_TOKEN_ID = 0

# Padding logprob. Large-negative rather than -inf so that a caller who wrongly
# exponentiates the whole row gets a zero contribution instead of a NaN.
PAD_LOGPROB = -65504.0

_ARRAY_NAMES = (
    "token_ids",
    "logprobs",
    "support_len",
    "tail_mass",
    "input_ids",
    "input_offset",
    "row_offset",
    "record_index",
    "completion_start",
)


class ArtifactError(Exception):
    """A shard is internally inconsistent and must not be trained on."""


@dataclass
class ShardBuilder:
    """Accumulates scored records into one shard's arrays.

    `top_k` is what was *requested* from the teacher; stored rows are `top_k + 1`
    wide to hold the appended actual token.
    """

    top_k: int
    _token_ids: list[list[int]] = field(default_factory=list)
    _logprobs: list[list[float]] = field(default_factory=list)
    _support_len: list[int] = field(default_factory=list)
    _input_ids: list[int] = field(default_factory=list)
    _input_offset: list[int] = field(default_factory=lambda: [0])
    _row_offset: list[int] = field(default_factory=lambda: [0])
    _record_index: list[int] = field(default_factory=list)
    _completion_start: list[int] = field(default_factory=list)

    @property
    def width(self) -> int:
        return self.top_k + 1

    @property
    def records(self) -> int:
        return len(self._record_index)

    @property
    def rows(self) -> int:
        return len(self._support_len)

    def add_record(
        self,
        record_index: int,
        token_ids: list[int],
        completion_start: int,
        distributions: list[list[tuple[int, float]]],
    ) -> None:
        """Add one record's tokens and the distributions at its scored positions.

        `distributions` holds one `(token_id, logprob)` list per supervised
        position, in order, and must be exactly as long as the completion.
        """
        expected = len(token_ids) - completion_start
        if expected <= 0:
            raise ArtifactError(f"record {record_index} has no scored positions")
        if len(distributions) != expected:
            raise ArtifactError(
                f"record {record_index} has {len(distributions)} distributions "
                f"for {expected} scored positions"
            )

        for support in distributions:
            if not support:
                raise ArtifactError(f"record {record_index} has an empty distribution")
            if len(support) > self.width:
                raise ArtifactError(
                    f"record {record_index} returned {len(support)} entries, "
                    f"more than the {self.width} a top-{self.top_k} request allows"
                )
            ids = [token_id for token_id, _ in support]
            logprobs = [logprob for _, logprob in support]
            pad = self.width - len(support)
            self._token_ids.append(ids + [PAD_TOKEN_ID] * pad)
            self._logprobs.append(logprobs + [PAD_LOGPROB] * pad)
            self._support_len.append(len(support))

        self._input_ids.extend(token_ids)
        self._input_offset.append(len(self._input_ids))
        self._row_offset.append(len(self._support_len))
        self._record_index.append(record_index)
        self._completion_start.append(completion_start)

    def to_arrays(self) -> dict[str, np.ndarray]:
        """Materialize the shard, computing tail mass in fp32 before storing fp16."""
        if not self.records:
            raise ArtifactError("cannot build an empty shard")
        logprobs = np.asarray(self._logprobs, dtype=np.float32)
        support_len = np.asarray(self._support_len, dtype=np.uint16)
        return {
            "token_ids": np.asarray(self._token_ids, dtype=np.uint32),
            "logprobs": logprobs.astype(np.float16),
            "support_len": support_len,
            "tail_mass": tail_mass(logprobs, support_len).astype(np.float16),
            "input_ids": np.asarray(self._input_ids, dtype=np.uint32),
            "input_offset": np.asarray(self._input_offset, dtype=np.uint32),
            "row_offset": np.asarray(self._row_offset, dtype=np.uint32),
            "record_index": np.asarray(self._record_index, dtype=np.uint32),
            "completion_start": np.asarray(self._completion_start, dtype=np.uint32),
        }


def tail_mass(logprobs: np.ndarray, support_len: np.ndarray) -> np.ndarray:
    """Probability the teacher assigned outside the stored support, per position.

    Summed over real entries only and computed in fp32: fp16 accumulation of up
    to `k+1` terms loses enough precision to push the total past 1.0 and turn the
    remainder negative. Floating-point noise can still do so by a hair, hence the
    clamp at zero — a negative tail is not a small error, it is an invalid
    distribution that would make the loss meaningless.
    """
    valid = np.arange(logprobs.shape[1])[None, :] < support_len[:, None].astype(np.int64)
    probs = np.where(valid, np.exp(logprobs.astype(np.float32)), 0.0)
    return np.clip(1.0 - probs.sum(axis=1), 0.0, 1.0)


def write_shard(path: str, arrays: dict[str, np.ndarray]) -> None:
    validate_shard(arrays)
    with open(path, "wb") as handle:
        np.savez_compressed(handle, **arrays)


def read_shard(path: str) -> dict[str, np.ndarray]:
    with np.load(path) as loaded:
        arrays = {name: loaded[name] for name in _ARRAY_NAMES}
    validate_shard(arrays)
    return arrays


def validate_shard(arrays: dict[str, np.ndarray]) -> None:
    """Fail loudly on any shard whose offsets do not describe its own contents.

    Every check here is an invariant a correct writer maintains, so a failure
    means the artifact is corrupt — and training on a corrupt shard produces a
    model that is quietly worse rather than a run that visibly breaks.
    """
    missing = [name for name in _ARRAY_NAMES if name not in arrays]
    if missing:
        raise ArtifactError(f"shard is missing arrays: {missing}")

    rows, width = arrays["token_ids"].shape
    if arrays["logprobs"].shape != (rows, width):
        raise ArtifactError("logprobs and token_ids disagree on shape")
    for name in ("support_len", "tail_mass"):
        if arrays[name].shape != (rows,):
            raise ArtifactError(f"{name} has {arrays[name].shape}, expected ({rows},)")

    support = arrays["support_len"].astype(np.int64)
    if support.size and (support.min() < 1 or support.max() > width):
        raise ArtifactError("support_len outside 1..width")

    records = arrays["record_index"].size
    for name in ("input_offset", "row_offset"):
        if arrays[name].size != records + 1:
            raise ArtifactError(f"{name} must have one more entry than records")
        offsets = arrays[name].astype(np.int64)
        if offsets[0] != 0 or np.any(np.diff(offsets) < 0):
            raise ArtifactError(f"{name} must start at 0 and be non-decreasing")
    if arrays["completion_start"].size != records:
        raise ArtifactError("completion_start must have one entry per record")

    if arrays["input_offset"][-1] != arrays["input_ids"].size:
        raise ArtifactError("input_offset does not end at the token count")
    if arrays["row_offset"][-1] != rows:
        raise ArtifactError("row_offset does not end at the row count")

    input_offset = arrays["input_offset"].astype(np.int64)
    row_offset = arrays["row_offset"].astype(np.int64)
    lengths = np.diff(input_offset)
    scored = np.diff(row_offset)
    completion_start = arrays["completion_start"].astype(np.int64)
    if np.any(completion_start >= lengths):
        raise ArtifactError("a record's completion starts at or past its end")
    if np.any(scored != lengths - completion_start):
        raise ArtifactError("scored rows do not match the supervised span")

    tail = arrays["tail_mass"].astype(np.float32)
    if tail.size and (tail.min() < 0.0 or tail.max() > 1.0):
        raise ArtifactError("tail_mass outside 0..1")


def record_view(arrays: dict[str, np.ndarray], position: int) -> dict[str, np.ndarray]:
    """Slice one record out of a shard by its position within that shard."""
    input_start = int(arrays["input_offset"][position])
    input_end = int(arrays["input_offset"][position + 1])
    row_start = int(arrays["row_offset"][position])
    row_end = int(arrays["row_offset"][position + 1])
    return {
        "input_ids": arrays["input_ids"][input_start:input_end],
        "completion_start": int(arrays["completion_start"][position]),
        "token_ids": arrays["token_ids"][row_start:row_end],
        "logprobs": arrays["logprobs"][row_start:row_end],
        "support_len": arrays["support_len"][row_start:row_end],
        "tail_mass": arrays["tail_mass"][row_start:row_end],
    }


@dataclass(frozen=True)
class ShardEntry:
    name: str
    records: int
    rows: int
    first_record_index: int


def build_manifest(
    *,
    top_k: int,
    teacher_model: str,
    teacher_revision: str,
    precision: str,
    tokenizer_hash: str,
    rendering_fingerprint: str,
    vllm_version: str,
    max_batch_tokens: int,
    created_at: str,
    shards: list[ShardEntry],
    skipped_records: int,
) -> dict:
    """Describe a dataset's artifacts so training can refuse mismatched ones.

    `tokenizer_hash` and `rendering_fingerprint` are the two values training
    re-derives and compares: together they answer "would my tokenizer and
    template produce the token ids these distributions were computed for?".
    `vllm_version` and `max_batch_tokens` are recorded because prompt logprobs
    are known to vary with batch composition, so a run that cannot be reproduced
    can at least be explained.
    """
    return {
        "version": 1,
        "top_k": top_k,
        "width": top_k + 1,
        "teacher": {
            "model": teacher_model,
            "revision": teacher_revision,
            "precision": precision,
        },
        "tokenizer_hash": tokenizer_hash,
        "rendering_fingerprint": rendering_fingerprint,
        "vllm_version": vllm_version,
        "max_batch_tokens": max_batch_tokens,
        "created_at": created_at,
        "skipped_records": skipped_records,
        "totals": {
            "records": sum(shard.records for shard in shards),
            "scored_positions": sum(shard.rows for shard in shards),
        },
        "shards": [
            {
                "name": shard.name,
                "records": shard.records,
                "rows": shard.rows,
                "first_record_index": shard.first_record_index,
            }
            for shard in shards
        ],
    }


def manifest_matches(manifest: dict, *, tokenizer_hash: str, rendering_fingerprint: str) -> bool:
    """Whether artifacts were computed for the tokenization a caller will use."""
    return (
        manifest.get("tokenizer_hash") == tokenizer_hash
        and manifest.get("rendering_fingerprint") == rendering_fingerprint
    )


def dump_manifest(manifest: dict) -> bytes:
    return json.dumps(manifest, indent=2, sort_keys=True).encode("utf-8")
