"""Extract the actionable message from a Temporal failure chain.

Temporal wraps activity failures in its own exception layers, so `str(err)` on
what a workflow catches yields "Activity task failed" — true but useless to the
person who has to fix it. The message worth showing is the innermost one.
"""

_MAX_DEPTH = 8
_FALLBACK = "Generation failed"

# Wrapper text that carries no information on its own.
_WRAPPER_PREFIXES = ("Activity task failed", "Child workflow", "Workflow execution failed")


def root_cause_message(error: BaseException, fallback: str = _FALLBACK) -> str:
    """Return the innermost meaningful message in an exception chain."""
    messages: list[str] = []
    current: BaseException | None = error

    for _ in range(_MAX_DEPTH):
        if current is None:
            break
        text = str(current).strip()
        if text and not text.startswith(_WRAPPER_PREFIXES):
            messages.append(text)
        # Temporal exposes `.cause`; Python sets `__cause__` on `raise ... from`.
        nxt = getattr(current, "cause", None) or current.__cause__
        current = nxt if isinstance(nxt, BaseException) else None

    return messages[-1] if messages else fallback
