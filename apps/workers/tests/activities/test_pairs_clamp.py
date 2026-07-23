from src.activities.generate_pairs import (
    MAX_PAIRS_PER_CHUNK,
    MIN_PAIRS_PER_CHUNK,
    clamp_pairs_per_chunk,
)


def test_in_range_value_passes_through():
    assert clamp_pairs_per_chunk(5) == 5


def test_oversized_value_is_capped():
    assert clamp_pairs_per_chunk(1_000_000) == MAX_PAIRS_PER_CHUNK


def test_undersized_value_is_floored():
    assert clamp_pairs_per_chunk(0) == MIN_PAIRS_PER_CHUNK
    assert clamp_pairs_per_chunk(-3) == MIN_PAIRS_PER_CHUNK
