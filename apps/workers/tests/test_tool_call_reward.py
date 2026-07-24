"""Tests for the verifiable GRPO tool-call reward.

The rubric is a training signal, so exact values matter: these pin the
extraction behavior and every scoring tier.
"""

import json

from src.activities.tool_call_reward import (
    extract_tool_calls,
    reference_call_names,
    score_tool_call_completion,
)

WEATHER_TOOLS = [
    {
        "type": "function",
        "function": {
            "name": "get_weather",
            "parameters": {
                "type": "object",
                "properties": {"city": {"type": "string"}, "unit": {"type": "string"}},
                "required": ["city"],
            },
        },
    }
]

WEATHER_REF = [
    {
        "id": "c1",
        "type": "function",
        "function": {"name": "get_weather", "arguments": '{"city": "Paris"}'},
    }
]


def call_block(payload: dict) -> str:
    return f"<tool_call>\n{json.dumps(payload)}\n</tool_call>"


class TestExtractToolCalls:
    def test_single_block(self):
        text = "Let me check.\n" + call_block(
            {"name": "get_weather", "arguments": {"city": "Paris"}}
        )
        assert extract_tool_calls(text) == [{"name": "get_weather", "arguments": {"city": "Paris"}}]

    def test_multiple_blocks(self):
        text = (
            call_block({"name": "a", "arguments": {}})
            + "\n"
            + call_block({"name": "b", "arguments": {}})
        )
        assert [c["name"] for c in extract_tool_calls(text)] == ["a", "b"]

    def test_openai_wrapped_call(self):
        text = call_block({"function": {"name": "f", "arguments": '{"x": 1}'}})
        assert extract_tool_calls(text) == [{"name": "f", "arguments": {"x": 1}}]

    def test_string_arguments_are_parsed(self):
        text = call_block({"name": "f", "arguments": '{"x": 1}'})
        assert extract_tool_calls(text)[0]["arguments"] == {"x": 1}

    def test_unparsable_arguments_become_none(self):
        text = call_block({"name": "f", "arguments": "{not json"})
        assert extract_tool_calls(text)[0]["arguments"] is None

    def test_missing_arguments_default_to_empty(self):
        text = call_block({"name": "f"})
        assert extract_tool_calls(text)[0]["arguments"] == {}

    def test_bare_json_object_without_block(self):
        text = json.dumps({"name": "f", "arguments": {}})
        assert extract_tool_calls(text) == [{"name": "f", "arguments": {}}]

    def test_malformed_block_is_dropped(self):
        assert extract_tool_calls("<tool_call>{oops</tool_call>") == []

    def test_missing_name_is_dropped(self):
        assert extract_tool_calls(call_block({"arguments": {}})) == []

    def test_plain_text_yields_nothing(self):
        assert extract_tool_calls("The weather is sunny today.") == []


class TestReferenceCallNames:
    def test_collects_names(self):
        assert reference_call_names(WEATHER_REF) == {"get_weather"}

    def test_empty_and_malformed(self):
        assert reference_call_names([]) == set()
        assert reference_call_names([{"no": "name"}]) == set()


class TestScoreRubric:
    def score(self, completion, tools=WEATHER_TOOLS, ref=WEATHER_REF):
        return score_tool_call_completion(completion, tools=tools, reference_calls=ref)

    def test_no_parsable_call_is_minus_one(self):
        assert self.score("It is sunny.") == -1.0

    def test_unknown_tool_name_scores_zero(self):
        completion = call_block({"name": "nuke_db", "arguments": {}})
        assert self.score(completion) == 0.0

    def test_valid_name_bad_args_scores_point_four(self):
        # Missing the required `city`.
        completion = call_block({"name": "get_weather", "arguments": {}})
        assert self.score(completion, ref=[]) == 0.4 + 0.3  # ref granted when absent

    def test_unknown_argument_key_fails_args_check(self):
        completion = call_block(
            {"name": "get_weather", "arguments": {"city": "Paris", "zip": "75001"}}
        )
        assert self.score(completion) == 0.4 + 0.3  # name + reference, args invalid

    def test_fully_valid_call_matching_reference_is_one(self):
        completion = call_block({"name": "get_weather", "arguments": {"city": "Paris"}})
        assert self.score(completion) == 1.0

    def test_reference_granted_when_no_reference(self):
        completion = call_block({"name": "get_weather", "arguments": {"city": "Paris"}})
        assert self.score(completion, ref=[]) == 1.0

    def test_empty_schema_grants_name_and_args(self):
        completion = call_block({"name": "anything", "arguments": {"x": 1}})
        assert self.score(completion, tools=[], ref=[]) == 1.0

    def test_empty_schema_reference_mismatch(self):
        completion = call_block({"name": "wrong_tool", "arguments": {}})
        assert self.score(completion, tools=[]) == 0.4 + 0.3

    def test_multiple_calls_average(self):
        good = call_block({"name": "get_weather", "arguments": {"city": "Paris"}})
        bad = call_block({"name": "nuke_db", "arguments": {}})
        assert self.score(good + "\n" + bad) == 0.5

    def test_tool_without_parameter_schema_accepts_any_args(self):
        tools = [{"type": "function", "function": {"name": "ping"}}]
        completion = call_block({"name": "ping", "arguments": {"whatever": True}})
        assert score_tool_call_completion(completion, tools=tools, reference_calls=[]) == 1.0


class TestGrpoRewardDispatch:
    def test_mixed_batch_routes_per_record(self):
        import src.activities.train_model as tm

        class StubJudge:
            def score_reasoning(self, completion):
                return 0.123

        reward = tm._build_grpo_reward(StubJudge())
        tool_completion = call_block({"name": "get_weather", "arguments": {"city": "Paris"}})
        rewards = reward(
            ["a plain reasoning answer", tool_completion],
            tools_json=["", json.dumps(WEATHER_TOOLS)],
            ref_calls_json=["", json.dumps(WEATHER_REF)],
        )
        assert rewards == [0.123, 1.0]

    def test_without_columns_everything_uses_judge(self):
        import src.activities.train_model as tm

        class StubJudge:
            def score_reasoning(self, completion):
                return 0.5

        reward = tm._build_grpo_reward(StubJudge())
        assert reward(["x", "y"]) == [0.5, 0.5]
