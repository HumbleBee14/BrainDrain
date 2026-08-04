"""Correctness of the teacher logprob artifact format.

The padding and tail-mass rules are the two places a plausible-looking shard
silently corrupts a training run, so they are tested against hand-computed
values rather than against the writer's own behavior.
"""

import math

import numpy as np
import pytest

from src.teacher.artifacts import (
    PAD_LOGPROB,
    PAD_TOKEN_ID,
    ArtifactError,
    ShardBuilder,
    ShardEntry,
    build_manifest,
    dump_manifest,
    manifest_matches,
    read_shard,
    record_view,
    tail_mass,
    validate_shard,
    write_shard,
)

TOP_K = 3


def distribution(*pairs):
    return list(pairs)


def build_one(top_k=TOP_K):
    """One record: 5 tokens, completion starting at 3, so 2 scored positions."""
    builder = ShardBuilder(top_k=top_k)
    builder.add_record(
        record_index=7,
        token_ids=[10, 11, 12, 13, 14],
        completion_start=3,
        distributions=[
            distribution((13, math.log(0.6)), (99, math.log(0.3))),
            distribution((14, math.log(0.5)), (98, math.log(0.2)), (97, math.log(0.1))),
        ],
    )
    return builder


def test_rows_are_the_scored_positions_only():
    arrays = build_one().to_arrays()

    assert arrays["token_ids"].shape == (2, TOP_K + 1)
    assert arrays["input_ids"].tolist() == [10, 11, 12, 13, 14]
    assert arrays["row_offset"].tolist() == [0, 2]
    assert arrays["completion_start"].tolist() == [3]


def test_width_holds_one_more_than_requested_top_k():
    """vLLM appends the actual token when it falls outside the top-k."""
    builder = ShardBuilder(top_k=2)
    builder.add_record(
        record_index=0,
        token_ids=[1, 2],
        completion_start=1,
        distributions=[distribution((2, math.log(0.1)), (50, math.log(0.7)), (51, math.log(0.15)))],
    )
    arrays = builder.to_arrays()

    assert arrays["token_ids"].shape == (1, 3)
    assert int(arrays["support_len"][0]) == 3


def test_more_entries_than_width_is_rejected():
    builder = ShardBuilder(top_k=1)
    with pytest.raises(ArtifactError, match="more than"):
        builder.add_record(
            record_index=0,
            token_ids=[1, 2],
            completion_start=1,
            distributions=[distribution((2, -1.0), (3, -2.0), (4, -3.0))],
        )


def test_short_support_is_padded_and_recorded():
    arrays = build_one().to_arrays()

    assert arrays["support_len"].tolist() == [2, 3]
    assert int(arrays["token_ids"][0][2]) == PAD_TOKEN_ID
    assert int(arrays["token_ids"][0][3]) == PAD_TOKEN_ID
    assert float(arrays["logprobs"][0][2]) == pytest.approx(PAD_LOGPROB, rel=1e-3)


def test_padding_contributes_no_probability_mass():
    """The invariant a loss depends on: padded entries must weigh nothing."""
    arrays = build_one().to_arrays()
    logprobs = arrays["logprobs"].astype(np.float32)
    support = arrays["support_len"].astype(np.int64)

    for row in range(logprobs.shape[0]):
        padded = np.exp(logprobs[row, support[row] :])
        assert padded.sum() == 0.0


def test_tail_mass_is_the_unstored_remainder():
    arrays = build_one().to_arrays()

    # Row 0 stores 0.6 + 0.3; row 1 stores 0.5 + 0.2 + 0.1.
    assert float(arrays["tail_mass"][0]) == pytest.approx(0.1, abs=2e-3)
    assert float(arrays["tail_mass"][1]) == pytest.approx(0.2, abs=2e-3)


def test_tail_mass_sums_only_real_support():
    """A padded row must not have its padding counted as probability."""
    logprobs = np.array([[math.log(0.4), PAD_LOGPROB, PAD_LOGPROB]], dtype=np.float32)
    support = np.array([1], dtype=np.uint16)

    assert tail_mass(logprobs, support)[0] == pytest.approx(0.6)


def test_tail_mass_clamps_floating_point_overshoot():
    """A distribution summing marginally over 1.0 must clamp, never go negative."""
    logprobs = np.array([[math.log(0.5), math.log(0.5000001), PAD_LOGPROB]], dtype=np.float32)
    support = np.array([2], dtype=np.uint16)

    assert tail_mass(logprobs, support)[0] == 0.0


def test_full_support_leaves_no_tail():
    logprobs = np.log(np.array([[0.25, 0.25, 0.25, 0.25]], dtype=np.float32))
    support = np.array([4], dtype=np.uint16)

    assert tail_mass(logprobs, support)[0] == pytest.approx(0.0, abs=1e-6)


def test_roundtrip_through_disk(tmp_path):
    arrays = build_one().to_arrays()
    path = str(tmp_path / "shard-000.npz")
    write_shard(path, arrays)

    loaded = read_shard(path)
    for name, expected in arrays.items():
        np.testing.assert_array_equal(loaded[name], expected)


def test_record_view_slices_one_record():
    builder = build_one()
    builder.add_record(
        record_index=8,
        token_ids=[20, 21, 22],
        completion_start=1,
        distributions=[
            distribution((21, math.log(0.9))),
            distribution((22, math.log(0.8))),
        ],
    )
    arrays = builder.to_arrays()

    second = record_view(arrays, 1)
    assert second["input_ids"].tolist() == [20, 21, 22]
    assert second["completion_start"] == 1
    assert second["token_ids"].shape[0] == 2
    assert second["support_len"].tolist() == [1, 1]


def test_distribution_count_must_match_the_supervised_span():
    builder = ShardBuilder(top_k=TOP_K)
    with pytest.raises(ArtifactError, match="distributions"):
        builder.add_record(
            record_index=0,
            token_ids=[1, 2, 3],
            completion_start=1,
            distributions=[distribution((2, -1.0))],
        )


def test_record_with_no_scored_positions_is_rejected():
    builder = ShardBuilder(top_k=TOP_K)
    with pytest.raises(ArtifactError, match="no scored positions"):
        builder.add_record(record_index=0, token_ids=[1, 2], completion_start=2, distributions=[])


def test_empty_shard_is_rejected():
    with pytest.raises(ArtifactError, match="empty shard"):
        ShardBuilder(top_k=TOP_K).to_arrays()


def test_validation_catches_offsets_that_lie():
    arrays = build_one().to_arrays()
    arrays["row_offset"] = np.asarray([0, 1], dtype=np.uint32)

    with pytest.raises(ArtifactError, match="row_offset does not end"):
        validate_shard(arrays)


def test_validation_catches_a_completion_past_the_end():
    arrays = build_one().to_arrays()
    arrays["completion_start"] = np.asarray([5], dtype=np.uint32)

    with pytest.raises(ArtifactError, match="completion starts at or past"):
        validate_shard(arrays)


def test_validation_catches_support_len_over_width():
    arrays = build_one().to_arrays()
    arrays["support_len"] = np.asarray([99, 3], dtype=np.uint16)

    with pytest.raises(ArtifactError, match="support_len outside"):
        validate_shard(arrays)


def test_validation_catches_a_missing_array():
    arrays = build_one().to_arrays()
    del arrays["tail_mass"]

    with pytest.raises(ArtifactError, match="missing arrays"):
        validate_shard(arrays)


def manifest():
    return build_manifest(
        top_k=TOP_K,
        teacher_model="Qwen/Qwen3-32B",
        teacher_revision="9216db5781bf21249d130ec9da846c4624c16137",
        precision="bf16",
        tokenizer_hash="tok-hash",
        rendering_fingerprint="render-hash",
        vllm_version="0.26.0",
        max_batch_tokens=8192,
        created_at="2026-08-04T00:00:00Z",
        shards=[ShardEntry(name="shard-000.npz", records=2, rows=4, first_record_index=0)],
        skipped_records=1,
    )


def test_manifest_totals_and_identity():
    built = manifest()

    assert built["width"] == TOP_K + 1
    assert built["totals"] == {"records": 2, "scored_positions": 4}
    assert built["skipped_records"] == 1
    assert built["teacher"]["precision"] == "bf16"


def test_manifest_only_matches_the_tokenization_it_was_built_for():
    built = manifest()

    assert manifest_matches(built, tokenizer_hash="tok-hash", rendering_fingerprint="render-hash")
    assert not manifest_matches(built, tokenizer_hash="other", rendering_fingerprint="render-hash")
    assert not manifest_matches(built, tokenizer_hash="tok-hash", rendering_fingerprint="other")


def test_manifest_serializes_stably():
    assert dump_manifest(manifest()) == dump_manifest(manifest())
