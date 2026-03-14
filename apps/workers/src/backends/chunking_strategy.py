"""Text chunking backend — swap the splitting algorithm without touching ChunkTextActivity.

Protocol: ChunkingStrategy
  chunk(text, chunk_size, overlap) -> list[str]

Built-in backends:
  "recursive"  — paragraph → sentence splitting with overlap (default)
  "sliding"    — fixed-size sliding window with token-approximate overlap

Register custom backends with register().
"""

from typing import Protocol


class ChunkingStrategy(Protocol):
    """Protocol for text chunking backends."""

    def chunk(self, text: str, chunk_size: int, overlap: int) -> list[str]:
        """Split text into chunks of approximately chunk_size chars with overlap."""
        ...


# -- Implementations --


class RecursiveChunkingStrategy:
    """Default chunking: split by paragraphs, then sentences, then add overlap."""

    def chunk(self, text: str, chunk_size: int, overlap: int) -> list[str]:
        if len(text) <= chunk_size:
            return [text] if text.strip() else []

        chunks: list[str] = []
        paragraphs = text.split("\n\n")
        current_chunk = ""

        for para in paragraphs:
            if len(current_chunk) + len(para) + 2 <= chunk_size:
                current_chunk += ("\n\n" if current_chunk else "") + para
            else:
                if current_chunk:
                    chunks.append(current_chunk.strip())
                if len(para) > chunk_size:
                    sentences = para.replace(". ", ".\n").split("\n")
                    current_chunk = ""
                    for sent in sentences:
                        if len(current_chunk) + len(sent) + 1 <= chunk_size:
                            current_chunk += (" " if current_chunk else "") + sent
                        else:
                            if current_chunk:
                                chunks.append(current_chunk.strip())
                            current_chunk = sent
                else:
                    current_chunk = para

        if current_chunk.strip():
            chunks.append(current_chunk.strip())

        # Add overlap between chunks
        if overlap > 0 and len(chunks) > 1:
            overlapped = [chunks[0]]
            for i in range(1, len(chunks)):
                prev_tail = chunks[i - 1][-overlap:] if len(chunks[i - 1]) > overlap else ""
                overlapped.append(prev_tail + " " + chunks[i] if prev_tail else chunks[i])
            return overlapped

        return chunks


class SlidingWindowChunkingStrategy:
    """Fixed-size sliding window chunking (character-based).

    Simpler and more predictable chunk sizes, good for uniform token budgets.
    """

    def chunk(self, text: str, chunk_size: int, overlap: int) -> list[str]:
        if len(text) <= chunk_size:
            return [text] if text.strip() else []

        chunks: list[str] = []
        step = max(1, chunk_size - overlap)
        pos = 0

        while pos < len(text):
            end = pos + chunk_size
            chunk = text[pos:end].strip()
            if chunk:
                chunks.append(chunk)
            pos += step

        return chunks


# -- Registry & factory --

_REGISTRY: dict[str, type] = {
    "recursive": RecursiveChunkingStrategy,
    "sliding": SlidingWindowChunkingStrategy,
}


def register(name: str, cls: type) -> None:
    """Register a custom ChunkingStrategy implementation."""
    _REGISTRY[name] = cls


def get(name: str) -> ChunkingStrategy:
    """Instantiate the named ChunkingStrategy."""
    cls = _REGISTRY.get(name)
    if cls is None:
        available = list(_REGISTRY)
        raise ValueError(f"Unknown chunking_backend '{name}'. Available: {available}")
    return cls()
