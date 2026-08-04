"""Correctness of the teacher scoring pass, without a GPU or a network.

The whole point of this job is that the distributions it stores line up with the
tokens the student will be supervised on, so the tests assert that alignment
against the actual token ids rather than against the writer's own bookkeeping.
A fake engine stands in for vLLM and reproduces the behavior measured in
docs/distillation/STAGE2-SPIKE-FINDINGS.md: `prompt_logprobs` aligned 1:1 with
the ids that were sent, position 0 the only `None`, and support of k or k+1.
"""

import json
import math
from dataclasses import dataclass

import numpy as np
import pytest

from src.activities.extract_logprobs import (
    TOKENIZER_MISMATCH_MESSAGE,
    ScoringError,
    TeacherScorer,
    completion_distributions,
    run_extract_logprobs_core,
    token_budget_batches,
)
from src.activities.stubs import ExtractTeacherLogprobsInput
from src.teacher import tokenizer_identity
from src.teacher.artifacts import read_shard, record_view
from src.teacher.rendering import render_record
from tests.test_teacher_rendering import FakeTokenizer, conversation

TOP_K = 3
BUCKET = "test-bucket"
DATASET_KEY = "datasets/t1/p1/d1.jsonl"


@pytest.fixture(autouse=True)
def _isolate_tokenizer_cache():
    tokenizer_identity.clear_cache()
    yield
    tokenizer_identity.clear_cache()


# ── Fakes ──


@dataclass
class FakeLogprob:
    logprob: float
    rank: int = 1


class FakeEngine:
    """Scores every position, mimicking vLLM's measured output shape.

    Positions whose token index is even get a support of exactly `top_k`; odd
    ones get `top_k + 1`, which is what vLLM returns when the actual token falls
    outside the top-k. Both must be accepted.
    """

    def __init__(self, top_k=TOP_K, fail_after_records=None):
        self.top_k = top_k
        self.scored_ids: list[list[int]] = []
        self._fail_after_records = fail_after_records

    def generate(self, prompts, sampling_params):
        results = []
        for prompt in prompts:
            token_ids = list(prompt["prompt_token_ids"])
            if (
                self._fail_after_records is not None
                and len(self.scored_ids) >= self._fail_after_records
            ):
                raise RuntimeError("simulated crash mid-run")
            self.scored_ids.append(token_ids)
            results.append(
                FakeResult(
                    prompt_token_ids=token_ids,
                    prompt_logprobs=_logprobs(token_ids, self.top_k),
                )
            )
        return results


@dataclass
class FakeResult:
    prompt_token_ids: list[int]
    prompt_logprobs: list[dict | None]


def _support(actual_token_id: int, top_k: int, extra: bool) -> dict[int, FakeLogprob]:
    """Top-k entries plus, when `extra`, the actual token appended outside them."""
    support = {}
    if not extra:
        support[actual_token_id] = FakeLogprob(math.log(0.5), rank=1)
    for offset in range(top_k - (0 if extra else 1)):
        support[900000 + offset] = FakeLogprob(math.log(0.1), rank=offset + 2)
    if extra:
        support[actual_token_id] = FakeLogprob(math.log(0.01), rank=top_k + 1)
    return support


def _logprobs(token_ids: list[int], top_k: int) -> list[dict | None]:
    """Nothing precedes the first token, so position 0 has no distribution."""
    return [
        None if position == 0 else _support(token_id, top_k, extra=position % 2 == 1)
        for position, token_id in enumerate(token_ids)
    ]


def fake_prompt(token_ids):
    return {"prompt_token_ids": list(token_ids)}


def scorer_for(engine) -> TeacherScorer:
    return TeacherScorer(
        llm=engine, sampling_params=object(), prompt_factory=fake_prompt, top_k=engine.top_k
    )


class FakeS3:
    """In-memory S3 with the surface the extraction job actually uses."""

    def __init__(self):
        self.objects: dict[str, bytes] = {}
        self.writes: list[str] = []

    def get_object(self, Bucket, Key):  # noqa: N803 — boto3 keyword casing
        if Key not in self.objects:
            raise KeyError(Key)
        return {"Body": _Body(self.objects[Key])}

    def put_object(self, Bucket, Key, Body, ContentType=""):  # noqa: N803
        self.objects[Key] = Body
        self.writes.append(Key)

    def upload_file(self, local_path, bucket, key):
        with open(local_path, "rb") as handle:
            self.objects[key] = handle.read()
        self.writes.append(key)

    def list_objects_v2(self, Bucket, Prefix, ContinuationToken=None):  # noqa: N803
        contents = [{"Key": key} for key in sorted(self.objects) if key.startswith(Prefix)]
        return {"Contents": contents, "IsTruncated": False}

    def delete_objects(self, Bucket, Delete):  # noqa: N803
        for entry in Delete["Objects"]:
            self.objects.pop(entry["Key"], None)


class _Body:
    def __init__(self, data: bytes):
        self._data = data

    def read(self) -> bytes:
        return self._data

    def iter_lines(self):
        yield from self._data.split(b"\n")


class FakeSettings:
    hf_token = ""
    metrics_backend = "null"
    redis_url = ""


def matching_fetcher(model_id, filename, revision, hf_token):
    """Both models ship the same tokenizer bytes — the compatible case."""
    if filename == "tokenizer.json":
        return json.dumps({"vocab": {"a": 1}}).encode("utf-8")
    return None


def diverging_fetcher(model_id, filename, revision, hf_token):
    if filename == "tokenizer.json":
        return json.dumps({"vocab": {"a": 1}, "model": model_id}).encode("utf-8")
    return None


def dataset_bytes(answers) -> bytes:
    lines = [json.dumps({"messages": conversation(answer=answer)}) for answer in answers]
    return "\n".join(lines).encode("utf-8")


def extraction_input(**overrides) -> ExtractTeacherLogprobsInput:
    defaults = {
        "tenant_id": "t1",
        "training_job_id": "job-1234-5678",
        "dataset_path": DATASET_KEY,
        "teacher_model": "vendor/teacher-32b",
        "teacher_revision": "abc123",
        "student_model": "vendor/student-7b",
        "top_k": TOP_K,
        "max_batch_tokens": 64,
        "max_sequence_tokens": 128,
    }
    return ExtractTeacherLogprobsInput(**{**defaults, **overrides})


async def run(s3, engine, *, input=None, fetcher=matching_fetcher):
    return await run_extract_logprobs_core(
        input or extraction_input(),
        s3=s3,
        s3_bucket=BUCKET,
        settings=FakeSettings(),
        tokenizer=FakeTokenizer(),
        scorer=scorer_for(engine) if engine is not None else None,
        artifact_fetcher=fetcher,
    )


def manifest_of(s3: FakeS3, output) -> dict:
    return json.loads(s3.objects[output.manifest_path].decode("utf-8"))


def shard_arrays(s3: FakeS3, output, name, tmp_path):
    local = tmp_path / name
    local.write_bytes(s3.objects[f"{output.artifact_prefix}{name}"])
    return read_shard(str(local))


# ── The measured vLLM contract ──


def test_the_none_at_position_zero_does_not_shift_the_window():
    record = render_record(FakeTokenizer(), conversation())
    result = FakeResult(
        prompt_token_ids=list(record.token_ids),
        prompt_logprobs=_logprobs(list(record.token_ids), TOP_K),
    )

    distributions = completion_distributions(result, record, TOP_K)

    assert result.prompt_logprobs[0] is None
    assert len(distributions) == record.completion_len
    for offset, support in enumerate(distributions):
        expected = record.token_ids[record.completion_start + offset]
        assert expected in [token_id for token_id, _ in support]


def test_support_of_k_and_k_plus_one_are_both_accepted():
    record = render_record(FakeTokenizer(), conversation())
    result = FakeResult(
        prompt_token_ids=list(record.token_ids),
        prompt_logprobs=_logprobs(list(record.token_ids), TOP_K),
    )

    sizes = {len(support) for support in completion_distributions(result, record, TOP_K)}

    assert sizes == {TOP_K, TOP_K + 1}


def test_support_wider_than_k_plus_one_is_rejected():
    record = render_record(FakeTokenizer(), conversation())
    logprobs = _logprobs(list(record.token_ids), TOP_K)
    logprobs[record.completion_start] = {i: FakeLogprob(-1.0) for i in range(TOP_K + 2)}

    with pytest.raises(ScoringError, match="more than"):
        completion_distributions(
            FakeResult(prompt_token_ids=list(record.token_ids), prompt_logprobs=logprobs),
            record,
            TOP_K,
        )


def test_missing_distribution_inside_the_window_is_rejected():
    record = render_record(FakeTokenizer(), conversation())
    logprobs = _logprobs(list(record.token_ids), TOP_K)
    logprobs[record.completion_start + 1] = None

    with pytest.raises(ScoringError, match="no distribution"):
        completion_distributions(
            FakeResult(prompt_token_ids=list(record.token_ids), prompt_logprobs=logprobs),
            record,
            TOP_K,
        )


def test_window_that_does_not_score_its_own_token_is_rejected():
    """The off-by-one detector: shifted distributions stop scoring their own token."""
    record = render_record(FakeTokenizer(), conversation())
    token_ids = list(record.token_ids)
    shifted = _logprobs(token_ids[1:] + token_ids[:1], TOP_K)

    with pytest.raises(ScoringError, match="misaligned"):
        completion_distributions(
            FakeResult(prompt_token_ids=token_ids, prompt_logprobs=shifted), record, TOP_K
        )


def test_ids_the_teacher_scored_must_be_the_ids_that_were_sent():
    record = render_record(FakeTokenizer(), conversation())
    logprobs = _logprobs(list(record.token_ids), TOP_K)

    with pytest.raises(ScoringError, match="different token ids"):
        completion_distributions(
            FakeResult(prompt_token_ids=[1, 2, 3], prompt_logprobs=logprobs), record, TOP_K
        )


def test_token_ids_not_text_are_sent_to_the_engine():
    engine = FakeEngine()
    record = render_record(FakeTokenizer(), conversation())

    scorer_for(engine).score([record])

    assert engine.scored_ids == [list(record.token_ids)]


# ── Batching ──


def test_batches_are_bounded_by_tokens_not_by_record_count():
    tokenizer = FakeTokenizer()
    short = render_record(tokenizer, conversation(answer="Blue."))
    long = render_record(tokenizer, conversation(answer=" ".join(["word"] * 40)))
    items = [(0, short), (1, short), (2, long)]

    batches = list(token_budget_batches(items, max_batch_tokens=len(short.token_ids) * 2))

    assert [len(batch) for batch in batches] == [2, 1]


def test_a_record_larger_than_the_budget_is_still_scored_alone():
    record = render_record(FakeTokenizer(), conversation())

    batches = list(token_budget_batches([(0, record)], max_batch_tokens=1))

    assert batches == [[(0, record)]]


# ── The job end to end ──


@pytest.mark.asyncio
async def test_stored_distributions_are_exactly_the_completion_window(tmp_path):
    s3 = FakeS3()
    s3.objects[DATASET_KEY] = dataset_bytes(["Blue is a primary colour.", "Red."])
    engine = FakeEngine()

    output = await run(s3, engine)

    manifest = manifest_of(s3, output)
    arrays = shard_arrays(s3, output, manifest["shards"][0]["name"], tmp_path)
    tokenizer = FakeTokenizer()
    for position, answer in enumerate(["Blue is a primary colour.", "Red."]):
        expected = render_record(tokenizer, conversation(answer=answer))
        view = record_view(arrays, position)
        assert view["input_ids"].tolist() == list(expected.token_ids)
        assert view["completion_start"] == expected.completion_start
        assert view["token_ids"].shape[0] == expected.completion_len
        # Row j must hold the distribution over the token AT completion_start + j.
        for offset in range(expected.completion_len):
            scored = expected.token_ids[expected.completion_start + offset]
            real = view["token_ids"][offset][: view["support_len"][offset]]
            assert scored in real.tolist()
    assert output.records == 2
    assert output.scored_positions == arrays["token_ids"].shape[0]


@pytest.mark.asyncio
async def test_manifest_records_the_identity_of_what_produced_the_artifacts():
    s3 = FakeS3()
    s3.objects[DATASET_KEY] = dataset_bytes(["Blue."])

    output = await run(s3, FakeEngine())

    manifest = manifest_of(s3, output)
    hashes = tokenizer_identity.compute_tokenizer_hashes(
        "vendor/teacher-32b", revision="abc123", fetcher=matching_fetcher
    )
    assert manifest["tokenizer_hash"] == hashes.combined_hash
    assert manifest["rendering_fingerprint"]
    assert manifest["top_k"] == TOP_K
    assert manifest["teacher"] == {
        "model": "vendor/teacher-32b",
        "revision": "abc123",
        "precision": "bf16",
    }


@pytest.mark.asyncio
async def test_manifest_is_written_after_every_shard():
    """It is the commit point: its presence declares the shards usable."""
    s3 = FakeS3()
    s3.objects[DATASET_KEY] = dataset_bytes(["Blue.", "Red.", "Green."])

    output = await run(s3, FakeEngine(), input=extraction_input(shard_target_bytes=1))

    shard_writes = [key for key in s3.writes if key.endswith(".npz")]
    assert len(shard_writes) == 3
    assert s3.writes.index(output.manifest_path) > s3.writes.index(shard_writes[-1])


@pytest.mark.asyncio
async def test_tokenizer_mismatch_refuses_without_scoring():
    from temporalio.exceptions import ApplicationError

    s3 = FakeS3()
    s3.objects[DATASET_KEY] = dataset_bytes(["Blue."])
    engine = FakeEngine()

    with pytest.raises(ApplicationError) as raised:
        await run(s3, engine, fetcher=diverging_fetcher)

    assert raised.value.non_retryable is True
    assert str(raised.value) == TOKENIZER_MISMATCH_MESSAGE
    assert engine.scored_ids == []
    assert s3.writes == []


@pytest.mark.asyncio
async def test_unrenderable_records_are_skipped_and_counted():
    s3 = FakeS3()
    good = json.dumps({"messages": conversation()})
    no_answer = json.dumps({"messages": [{"role": "user", "content": "no answer"}]})
    s3.objects[DATASET_KEY] = "\n".join([good, no_answer, "", good]).encode("utf-8")

    output = await run(s3, FakeEngine())

    assert output.records == 2
    assert output.skipped_records == 2
    assert manifest_of(s3, output)["skipped_records"] == 2


@pytest.mark.asyncio
async def test_records_longer_than_the_teacher_context_are_skipped():
    s3 = FakeS3()
    s3.objects[DATASET_KEY] = dataset_bytes(["Blue.", " ".join(["word"] * 40)])

    output = await run(s3, FakeEngine(), input=extraction_input(max_sequence_tokens=20))

    assert output.records == 1
    assert output.skipped_records == 1


@pytest.mark.asyncio
async def test_resume_skips_already_written_shards():
    s3 = FakeS3()
    s3.objects[DATASET_KEY] = dataset_bytes(["Blue.", "Red.", "Green."])
    # One record per batch as well as per shard, so the crash lands between two
    # scoring calls the way a real interruption would.
    one_shard_per_record = extraction_input(shard_target_bytes=1, max_batch_tokens=12)

    crashing = FakeEngine(fail_after_records=2)
    with pytest.raises(RuntimeError):
        await run(s3, crashing, input=one_shard_per_record)
    assert len(crashing.scored_ids) == 2

    resuming = FakeEngine()
    output = await run(s3, resuming, input=one_shard_per_record)

    # Only the third record is paid for a second time; the first two shards are
    # taken from the checkpoint.
    assert len(resuming.scored_ids) == 1
    manifest = manifest_of(s3, output)
    assert [shard["first_record_index"] for shard in manifest["shards"]] == [0, 1, 2]
    assert output.records == 3


@pytest.mark.asyncio
async def test_progress_markers_are_cleared_once_the_manifest_commits():
    s3 = FakeS3()
    s3.objects[DATASET_KEY] = dataset_bytes(["Blue.", "Red."])

    output = await run(s3, FakeEngine(), input=extraction_input(shard_target_bytes=1))

    assert not [key for key in s3.objects if key.startswith(f"{output.artifact_prefix}progress/")]


@pytest.mark.asyncio
async def test_committed_artifacts_are_reused_without_a_teacher():
    s3 = FakeS3()
    s3.objects[DATASET_KEY] = dataset_bytes(["Blue."])
    first = await run(s3, FakeEngine())

    # scorer=None would import vLLM, which is not installed here: reaching the
    # GPU path at all would fail this test rather than silently cost money.
    second = await run(s3, None)

    assert second.manifest_path == first.manifest_path
    assert second.records == first.records
    assert second.metrics == {"reused_artifacts": 1}


@pytest.mark.asyncio
async def test_artifacts_live_under_the_dataset_prefix():
    s3 = FakeS3()
    s3.objects[DATASET_KEY] = dataset_bytes(["Blue."])

    output = await run(s3, FakeEngine())

    assert output.artifact_prefix.startswith("datasets/t1/p1/d1-teacher-logprobs/")
    assert output.manifest_path == f"{output.artifact_prefix}manifest.json"


@pytest.mark.asyncio
async def test_a_different_top_k_gets_its_own_artifacts():
    s3 = FakeS3()
    s3.objects[DATASET_KEY] = dataset_bytes(["Blue."])

    narrow = await run(s3, FakeEngine(top_k=TOP_K))
    wide = await run(s3, FakeEngine(top_k=TOP_K + 1), input=extraction_input(top_k=TOP_K + 1))

    assert narrow.artifact_prefix != wide.artifact_prefix


@pytest.mark.asyncio
async def test_padding_beyond_the_support_is_never_counted(tmp_path):
    s3 = FakeS3()
    s3.objects[DATASET_KEY] = dataset_bytes(["Blue is a primary colour."])

    output = await run(s3, FakeEngine())

    manifest = manifest_of(s3, output)
    arrays = shard_arrays(s3, output, manifest["shards"][0]["name"], tmp_path)
    support = arrays["support_len"].astype(np.int64)
    assert support.min() >= TOP_K
    assert support.max() <= TOP_K + 1
    assert arrays["token_ids"].shape[1] == TOP_K + 1
