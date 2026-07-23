import pytest
from temporalio.testing import ActivityEnvironment

from src.activities.generate_pairs import generate_pairs_with_checkpoint
from src.activities.pair_checkpoint import PairCheckpoint
from src.datagen.protocols import FaithfulnessVerdict, GeneratedPair


class _Body:
    def __init__(self, data: bytes):
        self._data = data

    def read(self) -> bytes:
        return self._data


class FakeS3:
    """Minimal in-memory stand-in for the boto3 S3 client surface used by
    PairCheckpoint (list/get/put/delete)."""

    def __init__(self):
        self.store: dict[str, bytes] = {}

    def put_object(self, *, Bucket, Key, Body, ContentType=None):
        self.store[Key] = Body

    def get_object(self, *, Bucket, Key):
        return {"Body": _Body(self.store[Key])}

    def list_objects_v2(self, *, Bucket, Prefix, ContinuationToken=None):
        keys = sorted(k for k in self.store if k.startswith(Prefix))
        return {"Contents": [{"Key": k} for k in keys], "IsTruncated": False}

    def delete_objects(self, *, Bucket, Delete):
        for obj in Delete["Objects"]:
            self.store.pop(obj["Key"], None)


def test_checkpoint_roundtrip_and_clear():
    cp = PairCheckpoint(FakeS3(), "bucket", "pfx/")
    assert cp.load() == {}

    cp.save(0, [{"instruction": "q0", "response": "a0"}])
    cp.save(2, [])  # a "done but empty" chunk
    cp.save(1, [{"instruction": "q1", "response": "a1"}])

    loaded = cp.load()
    assert set(loaded) == {0, 1, 2}
    assert loaded[0][0]["instruction"] == "q0"
    assert loaded[2] == []

    cp.clear()
    assert cp.load() == {}


def _chunks(n: int) -> list[dict]:
    return [
        {"text": f"chunk-{i} " + "x" * 60, "doc_id": f"d{i}", "chunk_id": f"c{i}"} for i in range(n)
    ]


class _CrashOnceGenerator:
    """Succeeds per chunk, but raises once (a crash) on the chunk whose text
    starts with `crash_marker`, then never again. Records every chunk it
    actually generated so a test can assert nothing is regenerated on resume."""

    def __init__(self, crash_marker: str):
        self.crash_marker = crash_marker
        self.crashed = False
        self.generated: list[str] = []

    async def generate(self, *, chunk_text, task_type, guidance, facet, count, avoid):
        marker = chunk_text.split()[0]
        if not self.crashed and marker == self.crash_marker:
            self.crashed = True
            raise ValueError("simulated crash mid-run")
        self.generated.append(marker)
        return [GeneratedPair(prompt=f"q-{marker}", response=f"a-{marker}", source_text=chunk_text)]


@pytest.mark.asyncio
async def test_resume_skips_completed_chunks_no_dupes():
    chunks = _chunks(5)
    s3 = FakeS3()
    checkpoint = PairCheckpoint(s3, "bucket", "pfx/")
    generator = _CrashOnceGenerator(crash_marker="chunk-2")
    env = ActivityEnvironment()

    kwargs = dict(
        task_type="qna",
        guidance="",
        pairs_per_chunk=1,
        faithfulness_enabled=False,
    )

    # Attempt 1: crashes on chunk-2 after chunks 0 and 1 are checkpointed.
    with pytest.raises(ValueError, match="simulated crash"):
        await env.run(
            generate_pairs_with_checkpoint, chunks, [], generator, None, checkpoint, **kwargs
        )
    assert set(checkpoint.load()) == {0, 1}
    assert generator.generated == ["chunk-0", "chunk-1"]

    # Attempt 2: same checkpoint, generator no longer crashes. Chunks 0 and 1
    # must NOT be regenerated; the run completes chunks 2-4.
    records, dropped, failed = await env.run(
        generate_pairs_with_checkpoint, chunks, [], generator, None, checkpoint, **kwargs
    )

    assert failed == 0
    doc_ids = [r["doc_id"] for r in records]
    assert sorted(doc_ids) == ["d0", "d1", "d2", "d3", "d4"]  # all present, once each
    assert len(doc_ids) == len(set(doc_ids))  # no duplicates
    # chunk-0 / chunk-1 were generated exactly once (during attempt 1).
    assert generator.generated.count("chunk-0") == 1
    assert generator.generated.count("chunk-1") == 1
    assert generator.generated[2:] == ["chunk-2", "chunk-3", "chunk-4"]


class _AllGenerator:
    async def generate(self, *, chunk_text, task_type, guidance, facet, count, avoid):
        marker = chunk_text.split()[0]
        return [GeneratedPair(prompt=f"q-{marker}", response=f"a-{marker}", source_text=chunk_text)]


class _DropChunk2Scorer:
    """Marks chunk-2's pair unfaithful, everything else faithful."""

    async def score(self, *, pair, source_text):
        consistent = not pair.response.endswith("chunk-2")
        return FaithfulnessVerdict(consistent=consistent, score=1.0, reason="")


@pytest.mark.asyncio
async def test_per_chunk_gate_checkpoints_only_kept_pairs():
    chunks = _chunks(3)
    checkpoint = PairCheckpoint(FakeS3(), "bucket", "pfx/")
    env = ActivityEnvironment()

    records, dropped, failed = await env.run(
        generate_pairs_with_checkpoint,
        chunks,
        [],
        _AllGenerator(),
        _DropChunk2Scorer(),
        checkpoint,
        task_type="qna",
        guidance="",
        pairs_per_chunk=1,
        faithfulness_enabled=True,
    )

    assert dropped == 1
    assert sorted(r["doc_id"] for r in records) == ["d0", "d1"]
    # The dropped chunk is still checkpointed (as empty) so a retry skips it.
    loaded = checkpoint.load()
    assert set(loaded) == {0, 1, 2}
    assert loaded[2] == []
