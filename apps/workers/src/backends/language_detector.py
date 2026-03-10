"""Language detection backend — swap the library without touching parse_document.

Protocol: LanguageDetector
  detect(text) -> str | None   (ISO 639-1 code, e.g. "en", "fr", or None)

Built-in backends:
  "langdetect" — langdetect library (default)
  "null"       — always returns None (testing / opt-out)

Register custom backends with register().
"""

from typing import Protocol


class LanguageDetector(Protocol):
    """Protocol for language detection backends."""

    def detect(self, text: str) -> str | None:
        """Detect the language of text. Returns ISO 639-1 code or None."""
        ...


# -- Implementations --


class LangdetectDetector:
    """Language detection using the langdetect library (default)."""

    def detect(self, text: str) -> str | None:
        if not text or len(text) < 20:
            return None
        try:
            from langdetect import detect

            return detect(text[:5000])
        except Exception:  # LangDetectException or any import error
            return None


class NullDetector:
    """No-op detector — always returns None. Useful for testing or opt-out."""

    def detect(self, text: str) -> str | None:
        return None


# -- Registry & factory --

_REGISTRY: dict[str, type] = {
    "langdetect": LangdetectDetector,
    "null": NullDetector,
}


def register(name: str, cls: type) -> None:
    """Register a custom LanguageDetector implementation."""
    _REGISTRY[name] = cls


def get(name: str) -> LanguageDetector:
    """Instantiate the named LanguageDetector.

    Raises ValueError listing available backends if name is unknown.
    """
    cls = _REGISTRY.get(name)
    if cls is None:
        available = list(_REGISTRY)
        raise ValueError(f"Unknown language_detector_backend '{name}'. Available: {available}")
    return cls()
