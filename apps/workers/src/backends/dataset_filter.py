"""Dataset quality filtering and deduplication backends.

Protocols:
  DatasetFilter  — filter(pairs) -> pairs  (remove low-quality entries)
  Deduplicator   — deduplicate(pairs) -> pairs  (remove duplicates)

Built-in backends:
  Filters:      "heuristic" (default, length-based rules)
  Deduplicators: "hash" (default, MD5 content hash)

Register custom backends with register_filter() / register_deduplicator().
"""

import hashlib
from typing import Protocol


class DatasetFilter(Protocol):
    """Protocol for training data quality filtering."""

    def filter(self, pairs: list[dict]) -> list[dict]:
        """Remove low-quality pairs. Returns filtered list."""
        ...


class Deduplicator(Protocol):
    """Protocol for training data deduplication."""

    def deduplicate(self, pairs: list[dict]) -> list[dict]:
        """Remove duplicate pairs. Returns deduplicated list."""
        ...


# -- Filter implementations --


class HeuristicFilter:
    """Default filter: length-based rules for instruction/response quality.

    Configurable thresholds via constructor kwargs.
    """

    def __init__(
        self,
        min_instruction_len: int = 10,
        min_response_len: int = 20,
        max_response_len: int = 5000,
    ):
        self.min_instruction_len = min_instruction_len
        self.min_response_len = min_response_len
        self.max_response_len = max_response_len

    def filter(self, pairs: list[dict]) -> list[dict]:
        filtered = []
        for pair in pairs:
            instruction = pair.get("instruction", "")
            response = pair.get("response", "")

            if not instruction or not response:
                continue
            if len(response) < self.min_response_len:
                continue
            if len(response) > self.max_response_len:
                continue
            if len(instruction) < self.min_instruction_len:
                continue

            filtered.append(pair)

        return filtered


# -- Deduplicator implementations --


class HashDeduplicator:
    """Default deduplicator: exact-match via MD5 content hash."""

    def deduplicate(self, pairs: list[dict]) -> list[dict]:
        seen: set[str] = set()
        unique = []
        for pair in pairs:
            content = pair.get("instruction", "") + "|" + pair.get("response", "")
            h = hashlib.md5(content.encode()).hexdigest()  # noqa: S324
            if h not in seen:
                seen.add(h)
                unique.append(pair)
        return unique


# -- Registries & factories --

_FILTER_REGISTRY: dict[str, type] = {
    "heuristic": HeuristicFilter,
}

_DEDUP_REGISTRY: dict[str, type] = {
    "hash": HashDeduplicator,
}


def register_filter(name: str, cls: type) -> None:
    """Register a custom DatasetFilter implementation."""
    _FILTER_REGISTRY[name] = cls


def register_deduplicator(name: str, cls: type) -> None:
    """Register a custom Deduplicator implementation."""
    _DEDUP_REGISTRY[name] = cls


def get_filter(name: str) -> DatasetFilter:
    """Instantiate the named DatasetFilter."""
    cls = _FILTER_REGISTRY.get(name)
    if cls is None:
        available = list(_FILTER_REGISTRY)
        raise ValueError(f"Unknown dataset_filter_backend '{name}'. Available: {available}")
    return cls()


def get_deduplicator(name: str) -> Deduplicator:
    """Instantiate the named Deduplicator."""
    cls = _DEDUP_REGISTRY.get(name)
    if cls is None:
        available = list(_DEDUP_REGISTRY)
        raise ValueError(f"Unknown dedup_backend '{name}'. Available: {available}")
    return cls()
