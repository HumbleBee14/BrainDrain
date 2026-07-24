"""Tests for the shared chat-formatting helpers.

These guard the train/serve consistency contract: formatting must come from the
model's own chat template, a missing template must be filled (not silently
skipped), and prompt/response splitting must be structural (not string-based).
"""

import sys
import types

import pytest

from src.activities.chat_template import (
    CHATML_FALLBACK_TEMPLATE,
    ensure_chat_template,
    render_chat,
    split_prompt_and_response,
)


class _Tok:
    def __init__(self, chat_template=None):
        self.chat_template = chat_template
        self.calls = []

    def apply_chat_template(self, messages, tokenize=False, add_generation_prompt=False):
        self.calls.append({"tokenize": tokenize, "add_generation_prompt": add_generation_prompt})
        rendered = "".join(f"[{m['role']}]{m['content']}" for m in messages)
        if add_generation_prompt:
            rendered += "[gen]"
        return rendered


class TestEnsureChatTemplate:
    def test_installs_fallback_when_absent(self):
        tok = _Tok(chat_template=None)
        ensure_chat_template(tok)
        assert tok.chat_template == CHATML_FALLBACK_TEMPLATE

    def test_installs_fallback_when_empty_string(self):
        tok = _Tok(chat_template="")
        ensure_chat_template(tok)
        assert tok.chat_template == CHATML_FALLBACK_TEMPLATE

    def test_preserves_existing_template(self):
        tok = _Tok(chat_template="MODEL_OWN_TEMPLATE")
        ensure_chat_template(tok)
        assert tok.chat_template == "MODEL_OWN_TEMPLATE"

    def test_returns_the_tokenizer(self):
        tok = _Tok()
        assert ensure_chat_template(tok) is tok


class TestRenderChat:
    def test_delegates_to_tokenizer_template(self):
        tok = _Tok(chat_template="x")
        out = render_chat(tok, [{"role": "user", "content": "hi"}])
        assert out == "[user]hi"
        assert tok.calls[-1] == {"tokenize": False, "add_generation_prompt": False}

    def test_generation_prompt_flag(self):
        tok = _Tok(chat_template="x")
        out = render_chat(tok, [{"role": "user", "content": "hi"}], add_generation_prompt=True)
        assert out == "[user]hi[gen]"
        assert tok.calls[-1]["add_generation_prompt"] is True


class TestSplitPromptAndResponse:
    def test_simple_user_assistant(self):
        msgs = [
            {"role": "user", "content": "q"},
            {"role": "assistant", "content": "a"},
        ]
        prompt, gold = split_prompt_and_response(msgs)
        assert prompt == [{"role": "user", "content": "q"}]
        assert gold == "a"

    def test_system_user_assistant(self):
        msgs = [
            {"role": "system", "content": "s"},
            {"role": "user", "content": "q"},
            {"role": "assistant", "content": "a"},
        ]
        prompt, gold = split_prompt_and_response(msgs)
        assert prompt == [{"role": "system", "content": "s"}, {"role": "user", "content": "q"}]
        assert gold == "a"

    def test_multi_turn_splits_at_last_assistant(self):
        msgs = [
            {"role": "user", "content": "q1"},
            {"role": "assistant", "content": "a1"},
            {"role": "user", "content": "q2"},
            {"role": "assistant", "content": "a2"},
        ]
        prompt, gold = split_prompt_and_response(msgs)
        assert prompt[-1] == {"role": "user", "content": "q2"}
        assert gold == "a2"

    def test_empty_returns_none(self):
        assert split_prompt_and_response([]) == (None, None)

    def test_no_assistant_returns_none(self):
        assert split_prompt_and_response([{"role": "user", "content": "q"}]) == (None, None)

    def test_leading_assistant_only_returns_none(self):
        assert split_prompt_and_response([{"role": "assistant", "content": "a"}]) == (None, None)

    def test_empty_gold_returns_none(self):
        msgs = [{"role": "user", "content": "q"}, {"role": "assistant", "content": "  "}]
        assert split_prompt_and_response(msgs) == (None, None)


@pytest.fixture
def tm(monkeypatch):
    """Import train_model with a fake `datasets` module (ML-only dep, absent in dev)."""
    fake_datasets = types.ModuleType("datasets")

    class _DS:
        def __init__(self, rows):
            self.rows = rows

        @staticmethod
        def from_list(rows):
            return _DS(rows)

    fake_datasets.Dataset = _DS
    monkeypatch.setitem(sys.modules, "datasets", fake_datasets)
    import src.activities.train_model as tm

    return tm


class TestCreateGrpoPrompts:
    def test_templates_prompt_and_keeps_system(self, tm):
        tok = _Tok(chat_template="x")
        dataset = {
            "messages": [
                [
                    {"role": "system", "content": "be terse"},
                    {"role": "user", "content": "2+2?"},
                    {"role": "assistant", "content": "4"},
                ]
            ]
        }
        out = tm._create_grpo_prompts(dataset, tok)
        # Prompt is templated (has [gen] marker) and retains the system turn —
        # not the bare, system-dropped user string the old splitter produced.
        assert out.rows == [
            {"prompt": "[system]be terse[user]2+2?[gen]", "tools_json": "", "ref_calls_json": ""}
        ]

    def test_prompt_only_dataset_uses_whole_conversation(self, tm):
        tok = _Tok(chat_template="x")
        dataset = {"messages": [[{"role": "user", "content": "just a prompt"}]]}
        out = tm._create_grpo_prompts(dataset, tok)
        assert out.rows == [
            {"prompt": "[user]just a prompt[gen]", "tools_json": "", "ref_calls_json": ""}
        ]

    def test_tool_call_final_becomes_prompt_with_reference(self, tm):
        import json

        tok = _Tok(chat_template="x")
        calls = [
            {
                "id": "c1",
                "type": "function",
                "function": {"name": "get_weather", "arguments": "{}"},
            }
        ]
        dataset = {
            "messages": [
                [
                    {"role": "user", "content": "weather?"},
                    {"role": "assistant", "content": None, "tool_calls": calls},
                ]
            ]
        }
        out = tm._create_grpo_prompts(dataset, tok)
        assert len(out.rows) == 1
        row = out.rows[0]
        # The tool-call final is excluded from the prompt but preserved as the
        # reference the verifiable reward scores against.
        assert row["prompt"] == "[user]weather?[gen]"
        assert json.loads(row["ref_calls_json"]) == calls
        assert row["tools_json"] == ""

    def test_empty_final_without_tool_calls_is_skipped(self, tm):
        tok = _Tok(chat_template="x")
        dataset = {
            "messages": [
                [
                    {"role": "user", "content": "q"},
                    {"role": "assistant", "content": "  "},
                ]
            ]
        }
        out = tm._create_grpo_prompts(dataset, tok)
        assert out is dataset
