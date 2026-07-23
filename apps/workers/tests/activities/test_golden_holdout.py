"""Tests for the golden-holdout partition and golden eval-set formatting.

The golden set is generated from chunks the model never trains on, so
evaluation measures document knowledge instead of memorization of training
pairs. The partition must be deterministic (content-addressed, no RNG) so
activity retries and checkpoint resumes always agree on which chunks are
held out.
"""

from src.activities.build_dataset import _to_chat_records
from src.activities.generate_pairs import (
    MAX_HOLDOUT_RATIO,
    MIN_CHUNKS_FOR_HOLDOUT,
    select_holdout_chunks,
)


def _chunks(n: int) -> list[dict]:
    return [
        {"doc_id": "doc-1", "chunk_id": f"c{i}", "text": f"chunk text {i}" * 10} for i in range(n)
    ]


class TestSelectHoldoutChunks:
    def test_partition_is_disjoint_and_complete(self):
        chunks = _chunks(20)
        training, holdout = select_holdout_chunks(chunks, 0.1)

        assert len(training) + len(holdout) == len(chunks)
        held_ids = {c["chunk_id"] for c in holdout}
        train_ids = {c["chunk_id"] for c in training}
        assert held_ids.isdisjoint(train_ids)

    def test_ratio_respected(self):
        training, holdout = select_holdout_chunks(_chunks(20), 0.1)
        assert len(holdout) == 2  # round(20 * 0.1)

    def test_deterministic_across_calls(self):
        chunks = _chunks(30)
        _, holdout_a = select_holdout_chunks(chunks, 0.1)
        _, holdout_b = select_holdout_chunks(list(chunks), 0.1)
        assert [c["chunk_id"] for c in holdout_a] == [c["chunk_id"] for c in holdout_b]

    def test_below_chunk_floor_takes_no_holdout(self):
        chunks = _chunks(MIN_CHUNKS_FOR_HOLDOUT - 1)
        training, holdout = select_holdout_chunks(chunks, 0.1)
        assert holdout == []
        assert training == chunks

    def test_zero_ratio_disables_holdout(self):
        chunks = _chunks(50)
        training, holdout = select_holdout_chunks(chunks, 0.0)
        assert holdout == []
        assert training == chunks

    def test_ratio_capped(self):
        # A stray huge ratio must never starve training of its chunks.
        chunks = _chunks(100)
        training, holdout = select_holdout_chunks(chunks, 0.9)
        assert len(holdout) == round(100 * MAX_HOLDOUT_RATIO)
        assert len(training) == 100 - len(holdout)

    def test_fingerprint_falls_back_to_content_without_ids(self):
        # Chunks lacking doc/chunk ids still partition deterministically.
        chunks = [{"text": f"content {i}" * 20} for i in range(20)]
        _, holdout_a = select_holdout_chunks(chunks, 0.1)
        _, holdout_b = select_holdout_chunks([dict(c) for c in chunks], 0.1)
        assert len(holdout_a) == 2
        assert [c["text"] for c in holdout_a] == [c["text"] for c in holdout_b]

    def test_training_preserves_original_order(self):
        # Chunk order matters downstream (facet rotation, checkpoint indices).
        chunks = _chunks(20)
        training, _ = select_holdout_chunks(chunks, 0.1)
        positions = [chunks.index(c) for c in training]
        assert positions == sorted(positions)


class TestGoldenRecordFormatting:
    def test_formats_pairs_into_chat_records(self):
        pairs = [
            {
                "instruction": "What is X?",
                "response": "X is Y.",
                "doc_id": "d1",
                "chunk_id": "c1",
                "task_type": "qa",
            }
        ]
        records = _to_chat_records(pairs, "You are a helpful assistant.")
        assert len(records) == 1
        msgs = records[0]["messages"]
        assert [m["role"] for m in msgs] == ["system", "user", "assistant"]
        assert msgs[1]["content"] == "What is X?"
        assert records[0]["metadata"]["chunk_id"] == "c1"

    def test_skips_empty_instruction_or_response(self):
        pairs = [
            {"instruction": "", "response": "a"},
            {"instruction": "q", "response": "  "},
            {"instruction": "q", "response": "a"},
        ]
        records = _to_chat_records(pairs, "sys")
        assert len(records) == 1
