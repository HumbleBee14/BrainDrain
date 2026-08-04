"""Boundary bookkeeping for teacher scoring.

Uses a fake tokenizer so the tests are hermetic: a real chat template is not
needed to prove that the prompt/completion split is located correctly, and a
fake lets us construct the pathological merge-across-the-boundary case that a
real tokenizer will not produce on demand.
"""

import re

import pytest

from src.teacher.rendering import (
    RenderingError,
    count_tokens,
    render_dataset,
    render_record,
    rendering_fingerprint,
)

CHAT_TEMPLATE = (
    "{% for m in messages %}<|im_start|>{{ m['role'] }}\n{{ m['content'] }}<|im_end|>\n"
    "{% endfor %}{% if add_generation_prompt %}<|im_start|>assistant\n{% endif %}"
)


SPECIAL_MARKER = re.compile(r"(<\|[a-z_]+\|>)")


def fake_tokens(text):
    """Split on special markers first, then whitespace, as a real BPE would."""
    tokens = []
    for part in SPECIAL_MARKER.split(text):
        if not part:
            continue
        if part.startswith("<|"):
            tokens.append(part)
        else:
            tokens.extend(part.split())
    return tokens


class FakeTokenizer:
    """Word-per-token tokenizer that renders a ChatML-like template.

    Splits `<|...|>` markers into their own tokens, as every real tokenizer does
    for registered special tokens — the completion boundary lands between the
    answer and its end-of-turn marker, so gluing them would hide off-by-one bugs.
    """

    chat_template = CHAT_TEMPLATE

    def apply_chat_template(self, messages, tokenize=False, add_generation_prompt=False, **kwargs):
        parts = [f"<|im_start|>{m['role']}\n{m['content']}<|im_end|>\n" for m in messages]
        if add_generation_prompt:
            parts.append("<|im_start|>assistant\n")
        return "".join(parts)

    def __call__(self, text, add_special_tokens=True):
        tokens = fake_tokens(text)
        return {"input_ids": [len(token) * 31 + ord(token[0]) for token in tokens]}


class MergingTokenizer(FakeTokenizer):
    """Tokenizes the whole string as one token, so no prefix relationship holds."""

    def __call__(self, text, add_special_tokens=True):
        return {"input_ids": [len(text)]}


def conversation(answer="Blue is a primary colour."):
    return [
        {"role": "system", "content": "You are helpful."},
        {"role": "user", "content": "Name a primary colour."},
        {"role": "assistant", "content": answer},
    ]


def test_completion_starts_after_the_generation_prompt():
    tokenizer = FakeTokenizer()
    rendered = render_record(tokenizer, conversation())

    prompt_only = tokenizer.apply_chat_template(conversation()[:2], add_generation_prompt=True)
    expected_start = len(tokenizer(prompt_only)["input_ids"])

    assert rendered.completion_start == expected_start
    assert rendered.completion_len == len(rendered.token_ids) - expected_start
    assert rendered.completion_len > 0


def test_label_mask_covers_exactly_the_completion():
    rendered = render_record(FakeTokenizer(), conversation())
    mask = rendered.label_mask()

    assert len(mask) == len(rendered.token_ids)
    assert not any(mask[: rendered.completion_start])
    assert all(mask[rendered.completion_start :])
    assert sum(mask) == rendered.completion_len


def test_end_of_turn_marker_is_supervised():
    """The student must learn to stop, so the template's closing tokens count."""
    tokenizer = FakeTokenizer()
    rendered = render_record(tokenizer, conversation(answer="Blue."))
    answer_tokens = len(fake_tokens("Blue."))

    assert rendered.completion_len > answer_tokens


def test_record_without_assistant_turn_is_rejected():
    with pytest.raises(RenderingError):
        render_record(FakeTokenizer(), [{"role": "user", "content": "Hello?"}])


def test_record_with_empty_answer_is_rejected():
    with pytest.raises(RenderingError):
        render_record(FakeTokenizer(), conversation(answer="   "))


def test_boundary_that_cannot_be_located_is_rejected_not_guessed():
    with pytest.raises(RenderingError, match="not a prefix"):
        render_record(MergingTokenizer(), conversation())


def test_unusable_records_are_skipped_without_aborting_the_run():
    records = [
        {"messages": conversation()},
        {"messages": [{"role": "user", "content": "no answer"}]},
        {"messages": conversation(answer="Red.")},
    ]
    results = list(render_dataset(FakeTokenizer(), records))

    assert [index for index, _ in results] == [0, 1, 2]
    assert results[1][1] is None
    assert results[0][1] is not None and results[2][1] is not None


def test_counts_sum_over_usable_records_only():
    tokenizer = FakeTokenizer()
    good = {"messages": conversation()}
    bad = {"messages": []}
    counts = count_tokens(tokenizer, [good, bad, good])

    single = render_record(tokenizer, conversation())
    assert counts.records == 2
    assert counts.prompt_tokens == single.completion_start * 2
    assert counts.completion_tokens == single.completion_len * 2
    assert counts.scored_completion_tokens == counts.completion_tokens


def test_fingerprint_is_stable_and_template_sensitive():
    tokenizer = FakeTokenizer()
    assert rendering_fingerprint(tokenizer) == rendering_fingerprint(FakeTokenizer())

    class OtherTemplate(FakeTokenizer):
        def apply_chat_template(
            self, messages, tokenize=False, add_generation_prompt=False, **kwargs
        ):
            parts = [f"[{m['role']}] {m['content']}\n" for m in messages]
            if add_generation_prompt:
                parts.append("[assistant] ")
            return "".join(parts)

    assert rendering_fingerprint(OtherTemplate()) != rendering_fingerprint(tokenizer)


def test_fingerprint_detects_tokenizer_change_under_identical_template():
    class CoarserTokenizer(FakeTokenizer):
        def __call__(self, text, add_special_tokens=True):
            return {"input_ids": [len(chunk) for chunk in text.split("<|im_start|>")]}

    assert rendering_fingerprint(CoarserTokenizer()) != rendering_fingerprint(FakeTokenizer())
