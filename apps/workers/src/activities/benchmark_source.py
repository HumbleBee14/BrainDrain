"""BenchmarkSource protocol — abstracts benchmark dataset loading.

Default: LocalFileBenchmarkSource loads from bundled JSON files.
Custom sources can load from S3, HTTP, or databases.
"""

import json
import logging
from pathlib import Path
from typing import Protocol

logger = logging.getLogger("platform.benchmarks")


class BenchmarkSource(Protocol):
    """Protocol for loading benchmark datasets."""

    def load(self, name: str) -> list[dict]:
        """Load a benchmark by name. Returns list of question dicts."""
        ...


class LocalFileBenchmarkSource:
    """Load benchmarks from local JSON files (default)."""

    def __init__(self, base_dir: Path | None = None):
        self._base_dir = base_dir or (Path(__file__).parent / "benchmarks")

    def load(self, name: str) -> list[dict]:
        path = self._base_dir / name
        if not path.exists():
            raise FileNotFoundError(f"Benchmark not found: {path}")
        with open(path) as f:
            return json.load(f)


def get_default_source() -> LocalFileBenchmarkSource:
    """Get the default benchmark source."""
    return LocalFileBenchmarkSource()
