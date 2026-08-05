"""The model's answer, separated from the packaging it arrived in.

Two wrappers routinely surround the answer itself. Reasoning models (Qwen3,
R1-style) prepend a `<think>...</think>` block to every completion regardless
of what the prompt asked for. Instruction-tuned models often wrap structured
output in a markdown code fence even when told to respond with raw JSON.
Any code that parses model output — JSON payloads, single-letter verdicts —
must see the answer, not its packaging, so every consumer unwraps through
`answer_text` rather than each inventing its own handling.
"""

import re

_LEADING_REASONING = re.compile(r"\A\s*<think>.*?</think>\s*", re.DOTALL)

_WRAPPING_CODE_FENCE = re.compile(r"\A\s*```[\w-]*[ \t]*\n(.*)\n[ \t]*```\s*\Z", re.DOTALL)


def strip_reasoning(text: str) -> str:
    """Remove one complete leading `<think>...</think>` block, if present.

    Deliberately strict: only a complete block at the very start is removed.
    An unterminated block means the completion was cut off mid-reasoning and
    carries no answer — that text is returned unchanged so the caller's parse
    fails loudly instead of salvaging garbage. A block later in the text is
    part of the answer's content, not a preamble, and is left alone.
    """
    return _LEADING_REASONING.sub("", text, count=1)


def strip_code_fence(text: str) -> str:
    """Unwrap one markdown code fence, if it encloses the entire text.

    Same strictness as `strip_reasoning`: the fence is packaging only when it
    wraps the whole answer. A fence with trailing prose after it, one opened
    but never closed (truncation), or one appearing mid-text is content or
    damage — returned unchanged so the caller's parse fails loudly.
    """
    match = _WRAPPING_CODE_FENCE.match(text)
    return match.group(1) if match else text


def answer_text(text: str) -> str:
    """The answer with all recognized packaging removed.

    Reasoning comes off first — models that emit both put the think block
    outside the fence.
    """
    return strip_code_fence(strip_reasoning(text))
