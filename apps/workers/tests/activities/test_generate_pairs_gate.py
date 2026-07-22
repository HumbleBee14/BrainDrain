import httpx
import pytest
from temporalio.testing import ActivityEnvironment

from src.activities.generate_pairs import apply_faithfulness_gate, generate_pairs_for_chunks
from src.circuit_breaker import CircuitBreakerOpen
from src.datagen.protocols import FaithfulnessVerdict, GeneratedPair


@pytest.mark.asyncio
async def test_gate_drops_unfaithful_pairs():
    pairs = [
        GeneratedPair(prompt="q1", response="grounded", source_text="s"),
        GeneratedPair(prompt="q2", response="madeup", source_text="s"),
    ]

    class FakeScorer:
        async def score(self, *, pair, source_text):
            return FaithfulnessVerdict(
                consistent=(pair.response == "grounded"), score=1.0, reason=""
            )

    kept, dropped = await apply_faithfulness_gate(pairs, FakeScorer(), enabled=True)
    assert len(kept) == 1 and kept[0].prompt == "q1" and dropped == 1


@pytest.mark.asyncio
async def test_gate_disabled_keeps_all():
    pairs = [GeneratedPair(prompt="q", response="x", source_text="s")]
    kept, dropped = await apply_faithfulness_gate(pairs, None, enabled=False)
    assert len(kept) == 1 and dropped == 0


class _FlakyOnFirstChunkGenerator:
    """Raises a transient transport error on the first chunk, succeeds after."""

    def __init__(self, error: Exception):
        self.error = error
        self.calls = 0

    async def generate(self, *, chunk_text, task_type, guidance, facet, count, avoid):
        self.calls += 1
        if self.calls == 1:
            raise self.error
        return [GeneratedPair(prompt=f"q{self.calls}", response="a", source_text=chunk_text)]


class _AlwaysFailsGenerator:
    def __init__(self, error: Exception):
        self.error = error

    async def generate(self, *, chunk_text, task_type, guidance, facet, count, avoid):
        raise self.error


class _AlwaysFailsWithValueErrorGenerator:
    async def generate(self, *, chunk_text, task_type, guidance, facet, count, avoid):
        raise ValueError("LLM response was not valid JSON")


_TRANSIENT_ERRORS = [
    httpx.ConnectTimeout("timed out"),
    httpx.HTTPError("transport failed"),
    TimeoutError("op timed out"),
    CircuitBreakerOpen("circuit breaker 'llm' is open"),
]


@pytest.mark.asyncio
@pytest.mark.parametrize("error", _TRANSIENT_ERRORS)
async def test_transient_chunk_error_is_skipped_not_raised(error):
    chunks = [
        {"text": "x" * 60, "doc_id": "d1", "chunk_id": "c1"},
        {"text": "y" * 60, "doc_id": "d2", "chunk_id": "c2"},
    ]
    generator = _FlakyOnFirstChunkGenerator(error)

    env = ActivityEnvironment()
    generated, pair_meta, failed_chunks = await env.run(
        generate_pairs_for_chunks,
        chunks,
        [],
        generator,
        task_type="qna",
        guidance="",
        pairs_per_chunk=1,
    )

    assert failed_chunks == 1
    assert len(generated) == 1
    assert generated[0].prompt == "q2"
    assert pair_meta[id(generated[0])] == ("d2", "c2")


@pytest.mark.asyncio
@pytest.mark.parametrize("error", _TRANSIENT_ERRORS)
async def test_all_chunks_transiently_failing_raises(error):
    chunks = [
        {"text": "x" * 60, "doc_id": "d1", "chunk_id": "c1"},
        {"text": "y" * 60, "doc_id": "d2", "chunk_id": "c2"},
    ]
    generator = _AlwaysFailsGenerator(error)

    env = ActivityEnvironment()
    with pytest.raises(RuntimeError, match="failed synthetic pair generation"):
        await env.run(
            generate_pairs_for_chunks,
            chunks,
            [],
            generator,
            task_type="qna",
            guidance="",
            pairs_per_chunk=1,
        )


@pytest.mark.asyncio
async def test_value_error_from_generator_still_propagates():
    """Structured-parse/contract failures are not transient — fail loud."""
    chunks = [{"text": "x" * 60, "doc_id": "d1", "chunk_id": "c1"}]
    generator = _AlwaysFailsWithValueErrorGenerator()

    env = ActivityEnvironment()
    with pytest.raises(ValueError, match="not valid JSON"):
        await env.run(
            generate_pairs_for_chunks,
            chunks,
            [],
            generator,
            task_type="qna",
            guidance="",
            pairs_per_chunk=1,
        )
