"""Tests for the agent/tool-calling fine-tune track.

Imported tool trajectories (assistant `tool_calls`, `role: "tool"` turns,
top-level `tools` schemas) must survive dataset loading, render through the
chat template — including the ChatML fallback — and be handled (not crashed
on, not silently dropped) by the eval and DPO/GRPO paths.
"""

import json
import sys
import types

import pytest

from src.activities.chat_template import CHATML_FALLBACK_TEMPLATE, render_chat

# A canonical imported tool trajectory (OpenAI format, as stored by the
# JSONL import): assistant tool call (null content) -> tool result -> answer.
TOOL_MESSAGES = [
    {"role": "user", "content": "weather?"},
    {
        "role": "assistant",
        "content": None,
        "tool_calls": [
            {
                "id": "call_1",
                "type": "function",
                "function": {"name": "get_weather", "arguments": '{"city": "Paris"}'},
            }
        ],
    },
    {"role": "tool", "tool_call_id": "call_1", "content": "sunny"},
    {"role": "assistant", "content": "It is sunny."},
]

TOOLS = [{"type": "function", "function": {"name": "get_weather"}}]


class _Tok:
    """Tokenizer stub without a `tools` parameter — proves callers that pass no
    tools invoke `apply_chat_template` exactly as before (no extra kwarg)."""

    chat_template = "present"

    def __init__(self):
        self.calls = []

    def apply_chat_template(self, messages, tokenize=False, add_generation_prompt=False):
        self.calls.append({"tokenize": tokenize, "add_generation_prompt": add_generation_prompt})
        rendered = "".join(f"[{m['role']}]{m['content']}" for m in messages)
        if add_generation_prompt:
            rendered += "[gen]"
        return rendered


class _ToolTok:
    """Tokenizer stub that accepts and records the `tools` kwarg."""

    chat_template = "present"

    def __init__(self):
        self.tools_seen = []

    def apply_chat_template(
        self, messages, tokenize=False, add_generation_prompt=False, tools=None
    ):
        self.tools_seen.append(tools)
        rendered = "".join(f"[{m['role']}]" for m in messages)
        if tools:
            rendered = f"[tools:{len(tools)}]" + rendered
        if add_generation_prompt:
            rendered += "[gen]"
        return rendered


class TestRenderChatTools:
    def test_forwards_tools_when_given(self):
        tok = _ToolTok()
        out = render_chat(tok, TOOL_MESSAGES, tools=TOOLS)
        assert tok.tools_seen == [TOOLS]
        assert out.startswith("[tools:1]")

    def test_omits_tools_kwarg_when_none_or_empty(self):
        # _Tok has no `tools` parameter: passing the kwarg would raise TypeError.
        tok = _Tok()
        assert render_chat(tok, [{"role": "user", "content": "hi"}]) == "[user]hi"
        assert render_chat(tok, [{"role": "user", "content": "hi"}], tools=[]) == "[user]hi"


class TestFallbackTemplateToolRendering:
    """Render CHATML_FALLBACK_TEMPLATE with the same sandboxed environment the
    tokenizer runtime uses, and assert exact output."""

    @staticmethod
    def _render(messages, add_generation_prompt=False):
        from jinja2.sandbox import ImmutableSandboxedEnvironment

        env = ImmutableSandboxedEnvironment(trim_blocks=True, lstrip_blocks=True)
        return env.from_string(CHATML_FALLBACK_TEMPLATE).render(
            messages=messages, add_generation_prompt=add_generation_prompt
        )

    def test_plain_messages_render_exactly_as_before(self):
        out = self._render(
            [
                {"role": "system", "content": "be terse"},
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": "hello"},
            ]
        )
        assert out == (
            "<|im_start|>system\nbe terse<|im_end|>\n"
            "<|im_start|>user\nhi<|im_end|>\n"
            "<|im_start|>assistant\nhello<|im_end|>\n"
        )

    def test_generation_prompt_marker(self):
        out = self._render([{"role": "user", "content": "hi"}], add_generation_prompt=True)
        assert out == "<|im_start|>user\nhi<|im_end|>\n<|im_start|>assistant\n"

    def test_full_tool_trajectory(self):
        out = self._render(TOOL_MESSAGES)
        assert out == (
            "<|im_start|>user\nweather?<|im_end|>\n"
            "<|im_start|>assistant\n"
            "<tool_call>\n"
            '{"name": "get_weather", "arguments": {"city": "Paris"}}\n'
            "</tool_call><|im_end|>\n"
            "<|im_start|>tool\nsunny<|im_end|>\n"
            "<|im_start|>assistant\nIt is sunny.<|im_end|>\n"
        )

    def test_assistant_text_plus_tool_calls(self):
        out = self._render(
            [
                {
                    "role": "assistant",
                    "content": "Let me check.",
                    "tool_calls": [
                        {
                            "id": "c1",
                            "type": "function",
                            "function": {"name": "lookup", "arguments": "{}"},
                        }
                    ],
                }
            ]
        )
        assert out == (
            "<|im_start|>assistant\n"
            "Let me check.\n"
            "<tool_call>\n"
            '{"name": "lookup", "arguments": {}}\n'
            "</tool_call><|im_end|>\n"
        )

    def test_multiple_tool_calls_are_newline_separated(self):
        out = self._render(
            [
                {
                    "role": "assistant",
                    "content": None,
                    "tool_calls": [
                        {"function": {"name": "a", "arguments": "{}"}},
                        {"function": {"name": "b", "arguments": '{"x": 1}'}},
                    ],
                }
            ]
        )
        assert out == (
            "<|im_start|>assistant\n"
            '<tool_call>\n{"name": "a", "arguments": {}}\n</tool_call>\n'
            '<tool_call>\n{"name": "b", "arguments": {"x": 1}}\n</tool_call><|im_end|>\n'
        )


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


class TestLoadChatmlDataset:
    def test_keeps_tools_per_record(self, tm, tmp_path):
        path = tmp_path / "data.jsonl"
        with_tools = {"messages": TOOL_MESSAGES, "tools": TOOLS}
        without_tools = {"messages": [{"role": "user", "content": "q"}]}
        path.write_text(json.dumps(with_tools) + "\n\n" + json.dumps(without_tools) + "\n")

        ds = tm._load_chatml_dataset(path)

        assert ds.rows == [
            {"messages": TOOL_MESSAGES, "tools": TOOLS},
            {"messages": [{"role": "user", "content": "q"}], "tools": []},
        ]


class _FakeMapDataset:
    """Minimal stand-in for a HF Dataset: rows of dicts, `map`, `column_names`."""

    def __init__(self, rows):
        self.rows = rows

    @property
    def column_names(self):
        names = []
        for row in self.rows:
            for key in row:
                if key not in names:
                    names.append(key)
        return names

    def map(self, fn, remove_columns=None):
        out = []
        for row in self.rows:
            new = {**row, **fn(dict(row))}
            for col in remove_columns or []:
                new.pop(col, None)
            out.append(new)
        return _FakeMapDataset(out)


class TestRenderSftDataset:
    def test_tool_trajectory_renders_with_tools(self, tm):
        tok = _ToolTok()
        ds = _FakeMapDataset(
            [
                {"messages": TOOL_MESSAGES, "tools": TOOLS},
                {"messages": [{"role": "user", "content": "q"}], "tools": []},
            ]
        )

        out = tm._render_sft_dataset(ds, tok)

        assert out.rows == [
            {"text": "[tools:1][user][assistant][tool][assistant]"},
            {"text": "[user]"},
        ]


class TestGrpoPromptsToolSkips:
    def test_skips_tool_call_final_and_logs_count(self, tm, caplog):
        tok = _Tok()
        tool_final = [
            {"role": "user", "content": "weather?"},
            {
                "role": "assistant",
                "content": None,
                "tool_calls": [{"function": {"name": "get_weather", "arguments": "{}"}}],
            },
        ]
        normal = [
            {"role": "user", "content": "2+2?"},
            {"role": "assistant", "content": "4"},
        ]
        dataset = {"messages": [tool_final, normal]}

        with caplog.at_level("WARNING"):
            out = tm._create_grpo_prompts(dataset, tok)

        assert out.rows == [{"prompt": "[user]2+2?[gen]"}]
        assert "skipped 1" in caplog.text

    def test_prompt_only_records_still_fall_back(self, tm):
        tok = _Tok()
        dataset = {"messages": [[{"role": "user", "content": "just a prompt"}]]}
        out = tm._create_grpo_prompts(dataset, tok)
        assert out.rows == [{"prompt": "[user]just a prompt[gen]"}]

    def test_threads_tools_into_prompt(self, tm):
        tok = _ToolTok()
        dataset = {
            "messages": [[{"role": "user", "content": "weather?"}]],
            "tools": [TOOLS],
        }
        out = tm._create_grpo_prompts(dataset, tok)
        assert out.rows == [{"prompt": "[tools:1][user][gen]"}]


class TestEvalToolTrajectories:
    def test_prompt_and_expected_skips_tool_call_final(self):
        from src.activities.run_evaluation import _prompt_and_expected

        item = {
            "messages": [
                {"role": "user", "content": "weather?"},
                {
                    "role": "assistant",
                    "content": None,
                    "tool_calls": [{"function": {"name": "get_weather", "arguments": "{}"}}],
                },
            ]
        }
        assert _prompt_and_expected(item) is None

    def test_prompt_and_expected_skips_tool_result_final(self):
        from src.activities.run_evaluation import _prompt_and_expected

        item = {
            "messages": [
                {"role": "user", "content": "weather?"},
                {"role": "tool", "tool_call_id": "c1", "content": "sunny"},
            ]
        }
        assert _prompt_and_expected(item) is None

    def test_prompt_and_expected_returns_tools(self):
        from src.activities.run_evaluation import _prompt_and_expected

        split = _prompt_and_expected({"messages": TOOL_MESSAGES, "tools": TOOLS})
        assert split is not None
        prompt_msgs, expected, tools = split
        assert prompt_msgs == TOOL_MESSAGES[:-1]
        assert expected == "It is sunny."
        assert tools == TOOLS

    def test_domain_suite_skips_and_counts_tool_finals(self, monkeypatch):
        import src.activities.run_evaluation as re_mod
        from src.activities.run_evaluation import DomainSuite

        monkeypatch.setattr(re_mod, "_generate", lambda *a, **k: "generated")

        class _Judge:
            def score_domain(self, prompt, generated, expected):
                return {"accuracy": 4.0, "completeness": 4.0, "faithfulness": 4.0}

        val = [
            {"messages": TOOL_MESSAGES, "tools": TOOLS},  # valid: text final turn
            {
                "messages": [
                    {"role": "user", "content": "weather?"},
                    {
                        "role": "assistant",
                        "content": None,
                        "tool_calls": [{"function": {"name": "get_weather", "arguments": "{}"}}],
                    },
                ]
            },  # tool-call final: must be skipped, not crash on null content
        ]

        scores, report = DomainSuite().run(None, _ToolTok(), None, None, _Judge(), val)

        assert scores["mean"] == 4.0
        assert report["num_samples"] == 1
        assert report["skipped_samples"] == 1

    def test_format_prompt_forwards_tools(self):
        from src.activities.run_evaluation import _format_prompt

        tok = _ToolTok()
        out = _format_prompt(tok, [{"role": "user", "content": "weather?"}], tools=TOOLS)
        assert tok.tools_seen == [TOOLS]
        assert out == "[tools:1][user][gen]"


class TestDpoToolSkips:
    def test_tool_call_final_is_skipped_and_counted(self, monkeypatch, tm, caplog):
        fake_inf = types.SimpleNamespace(
            generate=lambda model, tok, prompt, max_new_tokens=256: "a sampled answer"
        )
        monkeypatch.setattr("src.backends.model_inference.get", lambda name="hf": fake_inf)

        class _Judge:
            def __init__(self, *a, **k):
                pass

            def compare_ab(self, prompt, a, b):
                return "A"

        monkeypatch.setattr(tm, "OpenAICompatibleJudge", _Judge)

        # Fake Dataset with from_dict for the DPO return value.
        class _DS:
            def __init__(self, d):
                self.d = d

            @staticmethod
            def from_dict(d):
                return _DS(d)

        sys.modules["datasets"].Dataset = _DS

        tool_final = [
            {"role": "user", "content": "weather?"},
            {
                "role": "assistant",
                "content": None,
                "tool_calls": [{"function": {"name": "get_weather", "arguments": "{}"}}],
            },
        ]
        normal = [
            {"role": "user", "content": "2+2?"},
            {"role": "assistant", "content": "4"},
        ]
        dataset = {"messages": [tool_final, normal], "tools": [TOOLS, []]}

        class _LlmConfig:
            api_base_url = "http://x"
            api_key = "k"
            model = "m"

        with caplog.at_level("WARNING"):
            out = tm._create_dpo_pairs(None, dataset, _ToolTok(), {}, _LlmConfig())

        assert len(out.d["chosen"]) == 1
        # The kept pair is the plain record — no tools marker in its rendering.
        assert "[tools:" not in out.d["chosen"][0]
        assert "skipped 1" in caplog.text
