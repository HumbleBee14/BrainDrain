"""Teacher logprob extraction — the GPU pass that produces distillation targets.

Runs a hosted teacher over an existing dataset once and stores its top-k
per-token distributions, so any number of later student runs train on them
without the teacher present. The on-disk format lives in
`src.teacher.artifacts`; where a completion begins lives in
`src.teacher.rendering`. This module is the job that joins the two to a vLLM
engine.

Three properties matter more than the code implementing them:

**The scored window is the completion.** vLLM's `prompt_logprobs[i]` is the
distribution over the token AT position `i` (measured — see
docs/distillation/STAGE2-SPIKE-FINDINGS.md), so the distributions that supervise
an answer are `plp[completion_start : completion_start + completion_len]`. An
off-by-one produces a run that trains on shifted targets and still looks
healthy, so the token actually present at each position is re-checked against
that position's support.

**Tokenizer identity is re-proven before any GPU work.** The API's eligibility
hint is advisory; this check is authoritative, and non-retryable, because a
tokenizer mismatch never becomes a match on a second attempt.

**Shards are the unit of progress.** Scoring is the expensive thing being
bought, so each shard is uploaded, then recorded, and a restart resumes at the
first record no shard covers rather than paying for it twice. `manifest.json` is
written last and is what declares the artifacts usable.
"""

import hashlib
import json
import logging
import os
import tempfile
import time
from collections.abc import Callable, Iterable, Iterator
from dataclasses import asdict, dataclass
from datetime import UTC, datetime

from temporalio import activity
from temporalio.exceptions import ApplicationError

from src.activities.stubs import ExtractTeacherLogprobsInput, ExtractTeacherLogprobsOutput
from src.gpu_provider import GpuProvider
from src.heartbeat import safe_heartbeat
from src.infra import InfraContainer
from src.teacher.artifacts import (
    ShardBuilder,
    ShardEntry,
    build_manifest,
    dump_manifest,
    manifest_matches,
    write_shard,
)
from src.teacher.rendering import RenderedRecord, render_dataset, rendering_fingerprint
from src.teacher.tokenizer_identity import check_tokenizer_identity

logger = logging.getLogger("platform.teacher.extract")

TOKENIZER_MISMATCH_MESSAGE = (
    "These two models read text differently, so high-fidelity training isn't "
    "possible between them. Standard distillation works — switch and re-run."
)

MANIFEST_NAME = "manifest.json"

_PROGRESS_DIR = "progress/"
_SHARD_SUFFIX = ".npz"
_METRICS_STREAM_PREFIX = "training:metrics:"
_METRICS_MAXLEN = 10000

# Uncompressed bytes one scored position costs: (k+1) entries of a uint32 token
# id plus an fp16 logprob, a uint16 support_len and an fp16 tail_mass, and the
# record's own uint32 input ids — counted twice per position so a dataset whose
# prompts are as long as its answers still lands inside the target size.
_BYTES_PER_ENTRY = 6
_BYTES_PER_ROW_SCALARS = 4
_BYTES_PER_INPUT_TOKEN = 8

# vLLM engine arguments per requested teacher precision. bf16 is the default
# because the product of this pass is the teacher's probability distribution and
# no published measurement says what fp8 does to logprobs specifically; the other
# two are knobs for someone trading fidelity for GPU time deliberately.
_PRECISION_ENGINE_ARGS: dict[str, dict[str, str]] = {
    "bf16": {"dtype": "bfloat16"},
    "fp8": {"dtype": "auto", "quantization": "fp8"},
    "int4": {"dtype": "bfloat16", "quantization": "bitsandbytes"},
}


class ScoringError(Exception):
    """The teacher's output does not line up with the tokens it was given.

    Always a bug in this file's bookkeeping or a change in vLLM's measured
    contract — never bad input data, which is skipped and counted instead.
    """


@dataclass
class TeacherScorer:
    """Turns a vLLM engine's prompt logprobs into per-position distributions.

    Deliberately holds no vLLM import: the caller builds the engine, the sampling
    params and the prompt wrapper, so every rule the spike measured is testable
    against a fake engine on a machine with no GPU.
    """

    llm: object
    sampling_params: object
    prompt_factory: Callable[[list[int]], object]
    top_k: int

    def score(self, batch: list[RenderedRecord]) -> list[list[list[tuple[int, float]]]]:
        prompts = [self.prompt_factory(list(record.token_ids)) for record in batch]
        results = self.llm.generate(prompts=prompts, sampling_params=self.sampling_params)
        if len(results) != len(batch):
            raise ScoringError(f"teacher returned {len(results)} results for {len(batch)} prompts")
        return [
            completion_distributions(result, record, self.top_k)
            for result, record in zip(results, batch, strict=True)
        ]


def completion_distributions(
    result, record: RenderedRecord, top_k: int
) -> list[list[tuple[int, float]]]:
    """The teacher's distributions at exactly the positions the student learns.

    Enforces the measured contract rather than trusting it: 1:1 alignment with
    the ids that were sent, no `None` inside the supervised window (only position
    0, which no completion contains, has no distribution), support of `k` or
    `k+1`, and the position's own token present in its support. The last check is
    what turns a silent target shift into a loud failure.
    """
    scored_ids = list(result.prompt_token_ids)
    if scored_ids != list(record.token_ids):
        raise ScoringError("teacher scored different token ids than were sent")

    prompt_logprobs = result.prompt_logprobs
    if len(prompt_logprobs) != len(record.token_ids):
        raise ScoringError(
            f"{len(prompt_logprobs)} distributions for {len(record.token_ids)} tokens"
        )

    width = top_k + 1
    distributions = []
    for offset in range(record.completion_len):
        position = record.completion_start + offset
        support = prompt_logprobs[position]
        if support is None:
            raise ScoringError(f"no distribution at supervised position {position}")
        if len(support) > width:
            raise ScoringError(
                f"position {position} returned {len(support)} entries, more than the "
                f"{width} a top-{top_k} request allows"
            )
        if record.token_ids[position] not in support:
            raise ScoringError(
                f"position {position} does not score its own token; "
                "the scored window is misaligned with the completion"
            )
        distributions.append(_ordered_support(support))
    return distributions


def _ordered_support(support) -> list[tuple[int, float]]:
    """Highest probability first, ties broken by token id.

    Row order then depends on the distribution rather than on dict iteration, so
    two runs over the same data produce comparable shards.
    """
    entries = [(int(token_id), float(entry.logprob)) for token_id, entry in support.items()]
    return sorted(entries, key=lambda pair: (-pair[1], pair[0]))


def token_budget_batches(
    records: Iterable[tuple[int, RenderedRecord]], max_batch_tokens: int
) -> Iterator[list[tuple[int, RenderedRecord]]]:
    """Group records so one batch never exceeds a token budget.

    Batching by tokens rather than by record count is what stops a handful of
    long conversations from setting the peak memory of the whole run. A single
    record longer than the budget goes alone — there is nothing smaller to split
    it into.
    """
    batch: list[tuple[int, RenderedRecord]] = []
    tokens = 0
    for index, record in records:
        length = len(record.token_ids)
        if batch and tokens + length > max_batch_tokens:
            yield batch
            batch, tokens = [], 0
        batch.append((index, record))
        tokens += length
    if batch:
        yield batch


def rows_per_shard(top_k: int, target_bytes: int) -> int:
    """Scored positions per shard, derived from the target file size.

    Sizing by positions rather than by a record count keeps shards the same size
    whether a dataset holds one-line answers or long transcripts. The estimate is
    of the uncompressed arrays, so `savez_compressed` lands the file at or under
    the target instead of over it.
    """
    per_row = (top_k + 1) * _BYTES_PER_ENTRY + _BYTES_PER_ROW_SCALARS + _BYTES_PER_INPUT_TOKEN
    return max(1, target_bytes // per_row)


def config_digest(
    *,
    teacher_model: str,
    teacher_revision: str,
    precision: str,
    top_k: int,
    tokenizer_hash: str,
    rendering_fingerprint: str,
) -> str:
    """Short id of everything that changes what a stored distribution means.

    Part of the artifact path, so artifacts scored under one teacher, precision,
    k or tokenization can never be resumed from — or trained on — as though they
    belonged to another.
    """
    raw = "|".join(
        [
            teacher_model,
            teacher_revision,
            precision,
            str(top_k),
            tokenizer_hash,
            rendering_fingerprint,
        ]
    )
    return hashlib.sha256(raw.encode("utf-8")).hexdigest()[:16]


def artifact_prefix(dataset_path: str, digest: str) -> str:
    """S3 prefix for one dataset's artifacts, under the dataset's own key.

    Keeping them there inherits the dataset's tenant scoping, deletion and
    tenant-erasure path; artifacts stored anywhere else would outlive the data
    they describe.
    """
    base = dataset_path[: -len(".jsonl")] if dataset_path.endswith(".jsonl") else dataset_path
    return f"{base}-teacher-logprobs/{digest}/"


@dataclass(frozen=True)
class ExtractionProgress:
    """Where a previous attempt of this same extraction got to."""

    shards: tuple[ShardEntry, ...]
    next_record_index: int
    skipped_records: int


class ShardProgress:
    """Which shards are already durable, recorded beside the artifacts.

    Modeled on pair generation's per-chunk checkpoint: one small object per
    completed unit, written only once that unit's real output is durable, and
    deleted when the manifest commits.
    """

    def __init__(self, s3, bucket: str, prefix: str):
        self._s3 = s3
        self._bucket = bucket
        self._prefix = f"{prefix}{_PROGRESS_DIR}"

    def load(self) -> ExtractionProgress:
        markers: dict[int, dict] = {}
        continuation: str | None = None
        while True:
            kwargs = {"Bucket": self._bucket, "Prefix": self._prefix}
            if continuation:
                kwargs["ContinuationToken"] = continuation
            response = self._s3.list_objects_v2(**kwargs)
            for obj in response.get("Contents", []):
                body = self._s3.get_object(Bucket=self._bucket, Key=obj["Key"])
                marker = json.loads(body["Body"].read().decode("utf-8"))
                markers[int(marker["ordinal"])] = marker
            if not response.get("IsTruncated"):
                break
            continuation = response.get("NextContinuationToken")
        return _contiguous_progress(markers)

    def save(
        self,
        ordinal: int,
        shard: ShardEntry,
        *,
        next_record_index: int,
        skipped_records: int,
    ) -> None:
        payload = {
            "ordinal": ordinal,
            "shard": asdict(shard),
            "next_record_index": next_record_index,
            "skipped_records": skipped_records,
        }
        self._s3.put_object(
            Bucket=self._bucket,
            Key=f"{self._prefix}{ordinal:05d}.json",
            Body=json.dumps(payload).encode("utf-8"),
            ContentType="application/json",
        )

    def clear(self) -> None:
        continuation: str | None = None
        while True:
            kwargs = {"Bucket": self._bucket, "Prefix": self._prefix}
            if continuation:
                kwargs["ContinuationToken"] = continuation
            response = self._s3.list_objects_v2(**kwargs)
            keys = [{"Key": obj["Key"]} for obj in response.get("Contents", [])]
            if keys:
                self._s3.delete_objects(Bucket=self._bucket, Delete={"Objects": keys})
            if not response.get("IsTruncated"):
                break
            continuation = response.get("NextContinuationToken")


def _contiguous_progress(markers: dict[int, dict]) -> ExtractionProgress:
    """Trust only an unbroken run of shards from the first one.

    A gap means the shard between them was never recorded, so its records are
    covered by nothing; resuming past it would silently drop them from the data
    the student trains on.
    """
    shards: list[ShardEntry] = []
    ordinal = 0
    next_record_index = 0
    skipped_records = 0
    while ordinal in markers:
        marker = markers[ordinal]
        shard = marker["shard"]
        shards.append(
            ShardEntry(
                name=shard["name"],
                records=int(shard["records"]),
                rows=int(shard["rows"]),
                first_record_index=int(shard["first_record_index"]),
            )
        )
        next_record_index = int(marker["next_record_index"])
        skipped_records = int(marker["skipped_records"])
        ordinal += 1
    return ExtractionProgress(
        shards=tuple(shards),
        next_record_index=next_record_index,
        skipped_records=skipped_records,
    )


@dataclass
class _SkipCounter:
    """Records the teacher could not score, by reason.

    `carried` is what an earlier attempt of this same extraction already counted,
    so the manifest's total covers the whole dataset and not just the part this
    attempt scored.
    """

    carried: int = 0
    unrenderable: int = 0
    too_long: int = 0

    @property
    def total(self) -> int:
        return self.carried + self.unrenderable + self.too_long


class ShardAccumulator:
    """Collects scored records and flushes them as shards of a target size.

    Owns the one ordering rule resume depends on: a shard's artifact is uploaded
    before its progress marker is written, so a marker never promises data that
    is not there.
    """

    def __init__(
        self,
        *,
        top_k: int,
        rows_target: int,
        s3,
        bucket: str,
        prefix: str,
        tmpdir: str,
        progress: ShardProgress,
        shards: list[ShardEntry],
        skipped: _SkipCounter,
    ):
        self._top_k = top_k
        self._rows_target = rows_target
        self._s3 = s3
        self._bucket = bucket
        self._prefix = prefix
        self._tmpdir = tmpdir
        self._progress = progress
        self._skipped = skipped
        self.shards = shards
        self._builder = ShardBuilder(top_k=top_k)
        self._first_record_index = 0
        self._next_record_index = 0

    @property
    def full(self) -> bool:
        return self._builder.rows >= self._rows_target

    @property
    def scored_records(self) -> int:
        return sum(shard.records for shard in self.shards) + self._builder.records

    def add(
        self,
        index: int,
        record: RenderedRecord,
        distributions: list[list[tuple[int, float]]],
    ) -> None:
        if self._builder.records == 0:
            self._first_record_index = index
        self._builder.add_record(
            index, list(record.token_ids), record.completion_start, distributions
        )
        self._next_record_index = index + 1

    def flush(self) -> ShardEntry | None:
        """Write and record the open shard; returns None when none is open."""
        if self._builder.records == 0:
            return None

        ordinal = len(self.shards)
        name = f"shard-{ordinal:05d}{_SHARD_SUFFIX}"
        local_path = os.path.join(self._tmpdir, name)
        write_shard(local_path, self._builder.to_arrays())
        self._s3.upload_file(local_path, self._bucket, f"{self._prefix}{name}")
        os.remove(local_path)

        entry = ShardEntry(
            name=name,
            records=self._builder.records,
            rows=self._builder.rows,
            first_record_index=self._first_record_index,
        )
        self._progress.save(
            ordinal,
            entry,
            next_record_index=self._next_record_index,
            skipped_records=self._skipped.total,
        )
        self.shards.append(entry)
        self._builder = ShardBuilder(top_k=self._top_k)
        return entry


async def run_extract_logprobs_core(
    input: ExtractTeacherLogprobsInput,
    *,
    s3,
    s3_bucket: str,
    settings,
    tokenizer=None,
    scorer: TeacherScorer | None = None,
    artifact_fetcher=None,
) -> ExtractTeacherLogprobsOutput:
    """Score one dataset with a hosted teacher and return where artifacts landed.

    Pure compute like the training core — S3 and the Hub, never Postgres — so it
    runs identically in-process and inside a Modal GPU container. `tokenizer`,
    `scorer` and `artifact_fetcher` are injection points that make the whole job
    testable without a GPU or a network.
    """
    tokenizer_hash = _verify_tokenizer_identity(input, settings, artifact_fetcher)
    if tokenizer is None:
        tokenizer = _load_tokenizer(input, settings)
    fingerprint = rendering_fingerprint(tokenizer)

    prefix = artifact_prefix(
        input.dataset_path,
        config_digest(
            teacher_model=input.teacher_model,
            teacher_revision=input.teacher_revision,
            precision=input.precision,
            top_k=input.top_k,
            tokenizer_hash=tokenizer_hash,
            rendering_fingerprint=fingerprint,
        ),
    )
    existing = _committed_manifest(s3, s3_bucket, prefix, tokenizer_hash, fingerprint)
    if existing is not None:
        logger.info("Teacher artifacts already committed at %s; skipping the GPU pass", prefix)
        return _output_from_manifest(prefix, existing, metrics={"reused_artifacts": 1})

    progress = ShardProgress(s3, s3_bucket, prefix)
    resume = progress.load()
    if resume.next_record_index:
        logger.info(
            "Resuming extraction at record %d after %d shards",
            resume.next_record_index,
            len(resume.shards),
        )

    load_started = time.monotonic()
    if scorer is None:
        scorer = _load_scorer(input)
    load_seconds = time.monotonic() - load_started
    _record_metric(
        settings, input.training_job_id, {"teacher_load_seconds": round(load_seconds, 2)}
    )

    with tempfile.TemporaryDirectory(prefix=f"extract-{input.training_job_id[:8]}-") as tmpdir:
        totals = _score_dataset(
            input,
            s3=s3,
            s3_bucket=s3_bucket,
            settings=settings,
            tokenizer=tokenizer,
            scorer=scorer,
            prefix=prefix,
            progress=progress,
            resume=resume,
            tmpdir=tmpdir,
        )

    if not totals.shards:
        raise ApplicationError(
            "None of this dataset's examples could be scored, so there is nothing "
            "to train on at higher fidelity. Standard distillation still works.",
            non_retryable=True,
        )

    manifest = _commit_manifest(
        input,
        s3=s3,
        s3_bucket=s3_bucket,
        prefix=prefix,
        tokenizer_hash=tokenizer_hash,
        fingerprint=fingerprint,
        totals=totals,
        progress=progress,
    )

    metrics = {
        "teacher_load_seconds": round(load_seconds, 2),
        "scoring_seconds": round(totals.seconds, 2),
        "scored_tokens_per_sec": _rate(totals.scored_tokens, totals.seconds),
        "shards": len(totals.shards),
    }
    _record_metric(settings, input.training_job_id, {"event": "complete", **metrics})
    logger.info(
        "Extraction complete: %d records, %d scored positions, %d shards, %d skipped",
        manifest["totals"]["records"],
        manifest["totals"]["scored_positions"],
        len(totals.shards),
        totals.skipped_records,
    )
    return _output_from_manifest(prefix, manifest, metrics=metrics)


def _commit_manifest(
    input: ExtractTeacherLogprobsInput,
    *,
    s3,
    s3_bucket: str,
    prefix: str,
    tokenizer_hash: str,
    fingerprint: str,
    totals: "_ScoringTotals",
    progress: ShardProgress,
) -> dict:
    """Write the manifest, then drop the progress markers it supersedes.

    This is the commit point of the whole job: until the manifest exists the
    shards beside it are not usable, and once it exists the per-shard markers are
    noise. Clearing them afterwards — never before — keeps a crash in between
    resumable rather than confusing.
    """
    manifest = build_manifest(
        top_k=input.top_k,
        teacher_model=input.teacher_model,
        teacher_revision=input.teacher_revision,
        precision=input.precision,
        tokenizer_hash=tokenizer_hash,
        rendering_fingerprint=fingerprint,
        vllm_version=_vllm_version(),
        max_batch_tokens=input.max_batch_tokens,
        created_at=datetime.now(UTC).isoformat(),
        shards=totals.shards,
        skipped_records=totals.skipped_records,
    )
    s3.put_object(
        Bucket=s3_bucket,
        Key=f"{prefix}{MANIFEST_NAME}",
        Body=dump_manifest(manifest),
        ContentType="application/json",
    )
    progress.clear()
    return manifest


@dataclass
class _ScoringTotals:
    shards: list[ShardEntry]
    skipped_records: int
    scored_tokens: int
    seconds: float


def _score_dataset(
    input: ExtractTeacherLogprobsInput,
    *,
    s3,
    s3_bucket: str,
    settings,
    tokenizer,
    scorer: TeacherScorer,
    prefix: str,
    progress: ShardProgress,
    resume: ExtractionProgress,
    tmpdir: str,
) -> _ScoringTotals:
    """Score every record from `resume` onward, shard by shard."""
    skipped = _SkipCounter(carried=resume.skipped_records)
    accumulator = ShardAccumulator(
        top_k=input.top_k,
        rows_target=rows_per_shard(input.top_k, input.shard_target_bytes),
        s3=s3,
        bucket=s3_bucket,
        prefix=prefix,
        tmpdir=tmpdir,
        progress=progress,
        shards=list(resume.shards),
        skipped=skipped,
    )
    scorable = _scorable_records(
        tokenizer,
        _stream_records(s3, s3_bucket, input.dataset_path, skip_lines=resume.next_record_index),
        first_index=resume.next_record_index,
        max_tokens=input.max_sequence_tokens,
        skipped=skipped,
    )

    scored_tokens = 0
    started = time.monotonic()
    for batch in token_budget_batches(scorable, input.max_batch_tokens):
        distributions = scorer.score([record for _, record in batch])
        for (index, record), support in zip(batch, distributions, strict=True):
            accumulator.add(index, record, support)
            scored_tokens += record.completion_len
            if accumulator.full:
                _flush_and_report(accumulator, input, settings, scored_tokens, started)
        safe_heartbeat(f"scored={accumulator.scored_records} shards={len(accumulator.shards)}")

    _flush_and_report(accumulator, input, settings, scored_tokens, started)
    return _ScoringTotals(
        shards=accumulator.shards,
        skipped_records=skipped.total,
        scored_tokens=scored_tokens,
        seconds=time.monotonic() - started,
    )


def _flush_and_report(
    accumulator: ShardAccumulator,
    input: ExtractTeacherLogprobsInput,
    settings,
    scored_tokens: int,
    started: float,
) -> None:
    entry = accumulator.flush()
    if entry is None:
        return
    elapsed = time.monotonic() - started
    _record_metric(
        settings,
        input.training_job_id,
        {
            "event": "shard",
            "shards": len(accumulator.shards),
            "records": accumulator.scored_records,
            "scored_positions": entry.rows,
            "scored_tokens_per_sec": _rate(scored_tokens, elapsed),
        },
    )
    logger.info(
        "Wrote %s (%d records, %d positions) at %s scored tokens/sec",
        entry.name,
        entry.records,
        entry.rows,
        _rate(scored_tokens, elapsed),
    )


def _scorable_records(
    tokenizer,
    records: Iterable[dict],
    *,
    first_index: int,
    max_tokens: int,
    skipped: _SkipCounter,
) -> Iterator[tuple[int, RenderedRecord]]:
    """Rendered records the teacher can score, counting the ones it cannot.

    One malformed or oversized example must not cost a GPU session that has
    already scored thousands of good ones, so it is skipped, logged and counted
    into the manifest instead of raising.
    """
    for offset, rendered in render_dataset(tokenizer, records):
        index = first_index + offset
        if rendered is None:
            skipped.unrenderable += 1
            continue
        if len(rendered.token_ids) > max_tokens:
            logger.warning(
                "Skipping record %d: %d tokens exceeds the teacher's %d-token context",
                index,
                len(rendered.token_ids),
                max_tokens,
            )
            skipped.too_long += 1
            continue
        yield index, rendered


def _stream_records(s3, bucket: str, key: str, *, skip_lines: int = 0) -> Iterator[dict]:
    """Yield a dataset's records one line at a time, skipping the first N lines.

    Streamed rather than downloaded whole because the scoring loop only ever
    holds one batch: buffering a multi-gigabyte dataset would be the largest
    allocation in a job whose memory belongs to the GPU. Skipping by line is what
    makes resume cheap — the records a previous attempt already scored are never
    parsed, let alone re-rendered.
    """
    body = s3.get_object(Bucket=bucket, Key=key)["Body"]
    for index, raw in enumerate(body.iter_lines()):
        if index < skip_lines:
            continue
        yield _parse_record(raw)


def _parse_record(raw) -> dict:
    """One dataset line as a record; an unusable line becomes an empty record.

    An empty record fails rendering and is counted as skipped, which keeps a
    record's index equal to its line number — the property resume relies on — and
    stops one corrupt line from discarding a paid scoring run.
    """
    text = raw.decode("utf-8", errors="replace") if isinstance(raw, bytes) else raw
    text = text.strip()
    if not text:
        return {}
    try:
        parsed = json.loads(text)
    except json.JSONDecodeError:
        logger.warning("Skipping a dataset line that is not valid JSON")
        return {}
    return parsed if isinstance(parsed, dict) else {}


def _verify_tokenizer_identity(input: ExtractTeacherLogprobsInput, settings, fetcher) -> str:
    """Prove teacher and student tokenize identically, or refuse to start.

    Non-retryable: two models that read text differently will still read it
    differently on the next attempt, and every retry would pay for the same
    Hub downloads to reach the same answer. A fetch *failure* is a different
    thing and stays retryable — it propagates untouched.
    """
    identity = check_tokenizer_identity(
        input.teacher_model,
        input.student_model,
        teacher_revision=input.teacher_revision or None,
        student_revision=input.student_revision or None,
        hf_token=settings.hf_token,
        fetcher=fetcher,
    )
    if not identity.compatible:
        logger.warning(
            "Tokenizer mismatch between %s and %s: %s",
            input.teacher_model,
            input.student_model,
            ", ".join(identity.mismatched_artifacts),
        )
        raise ApplicationError(TOKENIZER_MISMATCH_MESSAGE, non_retryable=True)
    return identity.teacher.combined_hash


def _committed_manifest(
    s3, bucket: str, prefix: str, tokenizer_hash: str, fingerprint: str
) -> dict | None:
    """A previous run's manifest, when it describes this exact tokenization.

    The manifest is the commit point, so its presence means every shard it lists
    is durable and the GPU pass can be skipped entirely. Any failure to read it
    means scoring again — the expensive outcome, never the wrong one.
    """
    try:
        body = s3.get_object(Bucket=bucket, Key=f"{prefix}{MANIFEST_NAME}")
        manifest = json.loads(body["Body"].read().decode("utf-8"))
    except Exception:
        return None
    if not manifest_matches(
        manifest, tokenizer_hash=tokenizer_hash, rendering_fingerprint=fingerprint
    ):
        logger.warning("Ignoring artifacts at %s: they describe a different tokenization", prefix)
        return None
    return manifest


def _output_from_manifest(
    prefix: str, manifest: dict, *, metrics: dict
) -> ExtractTeacherLogprobsOutput:
    totals = manifest["totals"]
    return ExtractTeacherLogprobsOutput(
        manifest_path=f"{prefix}{MANIFEST_NAME}",
        artifact_prefix=prefix,
        records=int(totals["records"]),
        scored_positions=int(totals["scored_positions"]),
        skipped_records=int(manifest["skipped_records"]),
        shards=len(manifest["shards"]),
        metrics=metrics,
    )


def _rate(count: int, seconds: float) -> float:
    return round(count / seconds, 1) if seconds > 0 else 0.0


def _load_tokenizer(input: ExtractTeacherLogprobsInput, settings):
    """Load the teacher's tokenizer at its pinned revision.

    The teacher's, not the student's: identity has just been proven, and it is
    the teacher that produced the ids the artifacts store. `trust_remote_code`
    stays off — we execute these files on our own GPUs.
    """
    from transformers import AutoTokenizer

    return AutoTokenizer.from_pretrained(
        input.teacher_model,
        revision=input.teacher_revision or None,
        token=settings.hf_token or None,
        trust_remote_code=False,
    )


def _load_scorer(input: ExtractTeacherLogprobsInput) -> TeacherScorer:
    """Bring up the teacher engine bound to the measured scoring contract.

    Every argument was paid for with GPU time (see the spike findings):
    `max_logprobs` is `k` and never uncapped (uncapped is an out-of-memory
    crash), `max_tokens=1` because the generated token is discarded and does not
    perturb the prompt logprobs, and ids rather than text are sent so the teacher
    scores exactly what `render_record` produced.
    """
    from vllm import LLM, SamplingParams

    engine_args = _PRECISION_ENGINE_ARGS.get(input.precision, _PRECISION_ENGINE_ARGS["bf16"])
    logger.info(
        "Loading teacher %s@%s (%s, top_k=%d)",
        input.teacher_model,
        input.teacher_revision or "default",
        input.precision,
        input.top_k,
    )
    llm = LLM(
        model=input.teacher_model,
        revision=input.teacher_revision or None,
        max_logprobs=input.top_k,
        max_model_len=input.max_sequence_tokens,
        trust_remote_code=False,
        **engine_args,
    )
    return TeacherScorer(
        llm=llm,
        sampling_params=SamplingParams(temperature=0.0, max_tokens=1, prompt_logprobs=input.top_k),
        prompt_factory=_tokens_prompt_factory(),
        top_k=input.top_k,
    )


def _tokens_prompt_factory() -> Callable[[list[int]], object]:
    """Resolve vLLM's token-ids prompt wrapper.

    Looked up at load time rather than imported at module scope: the worker that
    imports this module has no vLLM, and the tests run against a fake engine.
    Both documented locations are accepted because the class moved between vLLM
    releases and a wrong guess costs a GPU start to discover.
    """
    import importlib

    for module_name in ("vllm.inputs", "vllm"):
        try:
            module = importlib.import_module(module_name)
        except ImportError:
            continue
        prompt_class = getattr(module, "TokensPrompt", None)
        if prompt_class is not None:
            return lambda token_ids: prompt_class(prompt_token_ids=token_ids)
    raise RuntimeError("This vLLM build exposes no TokensPrompt; token-id scoring is unavailable")


def _vllm_version() -> str:
    """Installed vLLM version, recorded in the manifest.

    Prompt logprobs are known to vary with batch composition across versions, so
    a run that cannot be reproduced can at least be explained.
    """
    from importlib.metadata import PackageNotFoundError, version

    try:
        return version("vllm")
    except PackageNotFoundError:
        return "unknown"


_metrics_collector = None


def _record_metric(settings, job_id: str, data: dict) -> None:
    """Emit one extraction event onto the job's existing metrics stream.

    Same stream and sink the training run uses, tagged with the stage, so the
    dashboard shows scoring and training for one job side by side without new
    plumbing.
    """
    global _metrics_collector  # noqa: PLW0603

    try:
        if _metrics_collector is None:
            from src.backends.metrics_collector import get as get_collector

            _metrics_collector = get_collector(settings.metrics_backend, settings.redis_url)
        _metrics_collector.record(
            f"{_METRICS_STREAM_PREFIX}{job_id}",
            {"stage": "teacher_extraction", **{k: str(v) for k, v in data.items()}},
            maxlen=_METRICS_MAXLEN,
        )
    except Exception as exc:
        logger.warning("Failed to record extraction metric: %s", exc)


class ExtractTeacherLogprobsActivity:
    """Temporal entrypoint for the teacher scoring pass.

    Thin by design: the provider owns the GPU (local or Modal) and the core owns
    the compute, so this only dispatches and translates the result.
    """

    def __init__(self, infra: InfraContainer, gpu_provider: GpuProvider | None = None):
        self.infra = infra
        self.gpu_provider = gpu_provider

    @activity.defn(name="extract_teacher_logprobs")
    async def run(self, input: ExtractTeacherLogprobsInput) -> ExtractTeacherLogprobsOutput:
        if self.gpu_provider is None:
            return await run_extract_logprobs_core(
                input,
                s3=self.infra.s3,
                s3_bucket=self.infra.s3_bucket,
                settings=self.infra.settings,
            )

        result = await self.gpu_provider.extract_logprobs(
            tenant_id=input.tenant_id,
            training_job_id=input.training_job_id,
            gpu_class=input.gpu_class,
            extraction=asdict(input),
        )
        return ExtractTeacherLogprobsOutput(
            manifest_path=result["manifest_path"],
            artifact_prefix=result["artifact_prefix"],
            records=result["records"],
            scored_positions=result["scored_positions"],
            skipped_records=result["skipped_records"],
            shards=result["shards"],
            metrics=result["metrics"],
        )
