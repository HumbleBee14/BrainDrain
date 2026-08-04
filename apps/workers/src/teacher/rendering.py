"""Token-boundary bookkeeping shared by teacher scoring and student training.

Offline logit distillation supervises the student at the positions where the
teacher produced a distribution. If the two sides disagree about where the
assistant's answer starts by even one token, every target is shifted and the run
trains on noise while looking healthy — so the boundary is computed in exactly
one place, here, and both sides record `rendering_fingerprint` so a mismatch is
caught instead of silently absorbed.

The full example is rendered with the model's own chat template (the same
`render_chat` used for SFT), which is what makes the scored text byte-identical
to the text the student trains on.
"""

import hashlib
from collections.abc import Iterable, Iterator
from dataclasses import dataclass

from src.activities.chat_template import render_chat, split_prompt_and_response

# Rendered by `rendering_fingerprint` to detect template drift. Deliberately
# contains a system turn, a multi-line user turn and an assistant turn, so a
# template that changes how any of the three is wrapped changes the hash.
_FINGERPRINT_MESSAGES = [
    {"role": "system", "content": "You are a helpful assistant."},
    {"role": "user", "content": "First line.\nSecond line."},
    {"role": "assistant", "content": "Answer."},
]


class RenderingError(Exception):
    """A record cannot be given a reliable prompt/completion boundary."""


@dataclass(frozen=True)
class RenderedRecord:
    """One training example with the boundary the teacher scored it at."""

    token_ids: tuple[int, ...]
    completion_start: int

    @property
    def completion_len(self) -> int:
        return len(self.token_ids) - self.completion_start

    def label_mask(self) -> tuple[bool, ...]:
        """True at positions the student is supervised on."""
        return tuple(i >= self.completion_start for i in range(len(self.token_ids)))


@dataclass(frozen=True)
class TokenCounts:
    """Dataset totals under one tokenizer, used to price and admit extraction."""

    prompt_tokens: int
    completion_tokens: int
    records: int

    @property
    def scored_completion_tokens(self) -> int:
        """Positions a teacher would produce a distribution for.

        Equal to the completion tokens: the assistant's turn — including the
        template's end-of-turn marker, which the student must learn to emit — is
        exactly what gets supervised.
        """
        return self.completion_tokens


def rendering_fingerprint(tokenizer) -> str:
    """Hash of how `tokenizer` renders and tokenizes a fixed conversation.

    Covers more than the chat template string alone: an installed fallback
    template, a differing special-token map, or a tokenizer library that merges
    differently all change this value, which is the point — it answers "would
    these two sides produce the same token ids?" rather than "do these two
    template strings look the same?".
    """
    prompt_text = render_chat(tokenizer, _FINGERPRINT_MESSAGES[:2], add_generation_prompt=True)
    full_text = render_chat(tokenizer, _FINGERPRINT_MESSAGES, add_generation_prompt=False)
    prompt_ids = _encode(tokenizer, prompt_text)
    full_ids = _encode(tokenizer, full_text)
    payload = "\n".join(
        [
            full_text,
            ",".join(str(i) for i in prompt_ids),
            ",".join(str(i) for i in full_ids),
        ]
    )
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()


def render_record(tokenizer, messages: list[dict], tools: list | None = None) -> RenderedRecord:
    """Render one conversation and locate where the assistant's answer starts.

    Raises `RenderingError` when the record has no usable assistant turn, or when
    the prompt's tokens are not a prefix of the whole example's — the latter means
    the template or tokenizer merged tokens across the boundary, so no honest
    completion offset exists and guessing one would corrupt every target.
    """
    prompt_messages, gold = split_prompt_and_response(messages)
    if not prompt_messages or not gold:
        raise RenderingError("record has no assistant turn to learn from")

    prompt_text = render_chat(tokenizer, prompt_messages, add_generation_prompt=True, tools=tools)
    full_text = render_chat(tokenizer, messages, add_generation_prompt=False, tools=tools)

    prompt_ids = _encode(tokenizer, prompt_text)
    full_ids = _encode(tokenizer, full_text)
    if len(full_ids) <= len(prompt_ids) or full_ids[: len(prompt_ids)] != prompt_ids:
        raise RenderingError(
            "prompt tokens are not a prefix of the rendered example; "
            "this tokenizer merges across the assistant boundary"
        )

    return RenderedRecord(token_ids=tuple(full_ids), completion_start=len(prompt_ids))


def render_dataset(
    tokenizer, records: Iterable[dict]
) -> Iterator[tuple[int, RenderedRecord | None]]:
    """Render every record, yielding `(index, rendered_or_None)`.

    A record that cannot be given a boundary yields `None` rather than aborting
    the run: one malformed row in a large generated dataset should be skipped and
    counted, not cost the whole GPU session. Callers decide the tolerable rate.
    """
    for index, record in enumerate(records):
        try:
            yield index, render_record(tokenizer, record.get("messages", []), record.get("tools"))
        except RenderingError:
            yield index, None


def count_tokens(tokenizer, records: Iterable[dict]) -> TokenCounts:
    """Total prompt and completion tokens a teacher run would process."""
    prompt_tokens = 0
    completion_tokens = 0
    counted = 0
    for _, rendered in render_dataset(tokenizer, records):
        if rendered is None:
            continue
        prompt_tokens += rendered.completion_start
        completion_tokens += rendered.completion_len
        counted += 1
    return TokenCounts(
        prompt_tokens=prompt_tokens,
        completion_tokens=completion_tokens,
        records=counted,
    )


def _encode(tokenizer, text: str) -> list[int]:
    """Token ids for already-templated text.

    `add_special_tokens=False` because the chat template has already placed every
    special token; letting the tokenizer add its own would insert a second BOS
    that the teacher never scored.
    """
    return list(tokenizer(text, add_special_tokens=False)["input_ids"])
