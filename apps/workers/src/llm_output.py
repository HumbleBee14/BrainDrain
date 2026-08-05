"""The model's answer, separated from the reasoning it emitted first.

Reasoning models (Qwen3, R1-style) prepend a `<think>...</think>` block to
every completion, regardless of what the prompt asked for. The block is a
self-delimiting prefix: everything after it is the answer the prompt shaped.
Any code that parses model output — JSON payloads, single-letter verdicts —
must see the answer, not the preamble, so every consumer strips through this
one function rather than each inventing its own handling.
"""

import re

_LEADING_REASONING = re.compile(r"\A\s*<think>.*?</think>\s*", re.DOTALL)


def strip_reasoning(text: str) -> str:
    """Remove one complete leading `<think>...</think>` block, if present.

    Deliberately strict: only a complete block at the very start is removed.
    An unterminated block means the completion was cut off mid-reasoning and
    carries no answer — that text is returned unchanged so the caller's parse
    fails loudly instead of salvaging garbage. A block later in the text is
    part of the answer's content, not a preamble, and is left alone.
    """
    return _LEADING_REASONING.sub("", text, count=1)
