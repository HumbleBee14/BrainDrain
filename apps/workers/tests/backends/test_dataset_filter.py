from src.backends.dataset_filter import (
    HashDeduplicator,
    NearDuplicateDeduplicator,
    get_deduplicator,
)


def _pair(instruction: str, response: str) -> dict:
    return {"instruction": instruction, "response": response}


def test_hash_dedup_removes_only_exact_matches():
    pairs = [
        _pair("What is X?", "X is a thing."),
        _pair("What is X?", "X is a thing."),  # exact dup
        _pair("What is X exactly?", "X is a thing."),  # near, but not exact
    ]
    out = HashDeduplicator().deduplicate(pairs)
    assert len(out) == 2


def test_near_dedup_removes_paraphrases():
    pairs = [
        _pair("What is the capital of France?", "The capital of France is Paris."),
        # Reworded question, same answer → near-duplicate.
        _pair("What's the capital city of France?", "The capital of France is Paris."),
        _pair("What is the tallest mountain?", "Mount Everest is the tallest."),
    ]
    out = NearDuplicateDeduplicator(threshold=0.7).deduplicate(pairs)
    assert len(out) == 2
    assert out[0]["instruction"] == "What is the capital of France?"
    assert out[1]["instruction"] == "What is the tallest mountain?"


def test_near_dedup_keeps_distinct_pairs():
    pairs = [
        _pair("Explain photosynthesis.", "Plants convert light to energy."),
        _pair("Describe the water cycle.", "Water evaporates and condenses."),
    ]
    out = NearDuplicateDeduplicator().deduplicate(pairs)
    assert len(out) == 2


def test_registry_exposes_both_backends():
    assert isinstance(get_deduplicator("hash"), HashDeduplicator)
    assert isinstance(get_deduplicator("near"), NearDuplicateDeduplicator)
