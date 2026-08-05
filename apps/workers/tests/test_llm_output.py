"""What a parser sees when a reasoning model answers.

Qwen3 and R1-style teachers prepend `<think>...</think>` to every completion.
The first live datagen run against the platform's own catalog teacher failed
on exactly this: valid JSON, unreachable behind the preamble. These tests pin
the separation rule every output consumer relies on.
"""

import json

import pytest

from src.datagen.impls import _parse_json_object
from src.llm_output import answer_text, strip_code_fence, strip_reasoning

QWEN3_STYLE_COMPLETION = (
    "<think>\nAlright, let's tackle this query. The user wants Q&A pairs.\n"
    "I need to structure each pair into the required JSON format.\n</think>\n\n"
    '{"generated_qna_pairs": [{"query": "What are the six stages?", '
    '"answer": "Upload, parse, refine, train, evaluate, and deploy."}]}'
)

FENCED_VERDICT_COMPLETION = (
    '```json\n{"consistent": true, "score": 1.0, "reason": "The response correctly '
    "lists PDF, DOCX, HTML, and Markdown as supported formats, which are explicitly "
    "stated in the source text under the 'Upload and parsing' section.\"}\n```"
)


def test_the_answer_behind_a_reasoning_block_is_reachable():
    stripped = strip_reasoning(QWEN3_STYLE_COMPLETION)

    assert json.loads(stripped)["generated_qna_pairs"][0]["query"] == "What are the six stages?"


def test_a_completion_with_no_reasoning_block_passes_through_unchanged():
    assert strip_reasoning('{"verdict": "A"}') == '{"verdict": "A"}'


def test_an_unterminated_block_is_a_cut_off_completion_not_an_answer():
    """Truncation mid-reasoning leaves no answer to salvage; the text goes
    through unchanged so the caller's parse fails loudly."""
    truncated = "<think>\nreasoning that never finishe"

    assert strip_reasoning(truncated) == truncated


def test_a_think_tag_inside_the_answer_is_content_not_preamble():
    answer = '{"answer": "wrap reasoning in <think></think> tags"}'

    assert strip_reasoning(answer) == answer


def test_only_the_leading_block_is_removed():
    text = "<think>a</think>real answer <think>quoted</think> tail"

    assert strip_reasoning(text) == "real answer <think>quoted</think> tail"


def test_a_fenced_verdict_is_unwrapped():
    """The exact failure from the second live run: the faithfulness judge wrote
    a valid verdict wrapped in a markdown code fence."""
    data = json.loads(strip_code_fence(FENCED_VERDICT_COMPLETION))

    assert data["consistent"] is True


def test_a_fence_without_a_language_tag_is_unwrapped():
    assert strip_code_fence('```\n{"a": 1}\n```') == '{"a": 1}'


def test_unfenced_text_passes_through_unchanged():
    assert strip_code_fence('{"a": 1}') == '{"a": 1}'


def test_a_fence_with_trailing_prose_is_content_not_packaging():
    text = '```json\n{"a": 1}\n```\nHope that helps!'

    assert strip_code_fence(text) == text


def test_an_unterminated_fence_is_a_cut_off_completion_not_an_answer():
    truncated = '```json\n{"a": 1'

    assert strip_code_fence(truncated) == truncated


def test_a_fence_inside_the_answer_is_left_alone():
    text = 'Use ```json\n{"a": 1}\n``` to format it'

    assert strip_code_fence(text) == text


def test_answer_text_removes_reasoning_then_fence():
    completion = '<think>\nformatting now\n</think>\n```json\n{"a": 1}\n```'

    assert json.loads(answer_text(completion)) == {"a": 1}


def test_the_datagen_parser_reads_a_reasoning_teachers_pairs():
    """The exact failure from the first live run: Qwen3-32B wrote valid pairs
    the parser never saw."""
    data = _parse_json_object(QWEN3_STYLE_COMPLETION, required_keys=("generated_qna_pairs",))

    assert len(data["generated_qna_pairs"]) == 1


def test_the_datagen_parser_reads_a_fenced_verdict():
    data = _parse_json_object(FENCED_VERDICT_COMPLETION, required_keys=("consistent", "score"))

    assert data["score"] == 1.0


def test_the_datagen_parser_still_refuses_malformed_output():
    with pytest.raises(ValueError, match="not valid JSON"):
        _parse_json_object("<think>done</think>not json at all", required_keys=("x",))
