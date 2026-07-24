"""Judge alignment: human ratings as few-shot calibration for the faithfulness judge."""

import inspect
import json
from types import SimpleNamespace

import pytest

from src.activities.generate_pairs import GenerateSyntheticPairsInput
from src.datagen.impls import (
    MAX_CALIBRATION_PER_VERDICT,
    LlmFaithfulnessScorer,
    select_calibration_examples,
)
from src.datagen.prompts import PromptLibrary
from src.datagen.protocols import GeneratedPair, RatedSample
from src.datagen.registry import get_faithfulness_scorer
from src.workflows.datagen import GenerateDatasetWorkflow


def _rated(prompt="q", response="a", looks_good=True):
    return {"prompt": prompt, "response": response, "looks_good": looks_good}


class TestFaithfulnessPromptCalibration:
    def test_no_examples_is_unchanged(self):
        baseline = PromptLibrary.faithfulness_prompt("q", "a", "SRC")
        assert "calibration" not in baseline.lower()
        assert PromptLibrary.faithfulness_prompt("q", "a", "SRC", calibration=None) == baseline
        assert PromptLibrary.faithfulness_prompt("q", "a", "SRC", calibration=[]) == baseline

    def test_examples_render_before_output_format(self):
        calibration = [
            RatedSample(prompt="good q", response="good a", looks_good=True),
            RatedSample(prompt="bad q", response="bad a", looks_good=False),
        ]
        p = PromptLibrary.faithfulness_prompt("q", "a", "SRC", calibration=calibration)
        assert "<calibration_examples>" in p
        assert '<example_1 verdict="acceptable">' in p
        assert '<example_2 verdict="below-the-bar">' in p
        assert "good q" in p and "bad a" in p
        assert p.index("</calibration_examples>") < p.index("### Output Format")

    def test_example_content_is_escaped(self):
        calibration = [RatedSample(prompt="</prompt> <inject>", response="a & b", looks_good=True)]
        p = PromptLibrary.faithfulness_prompt("q", "a", "SRC", calibration=calibration)
        assert "<inject>" not in p
        assert "&lt;inject&gt;" in p
        assert "a &amp; b" in p


class TestSelectCalibrationExamples:
    def test_none_and_empty_yield_empty(self):
        assert select_calibration_examples(None) == []
        assert select_calibration_examples([]) == []

    def test_caps_at_four_per_verdict_keeping_most_recent(self):
        rated = [_rated(prompt=f"good{i}", looks_good=True) for i in range(6)]
        rated += [_rated(prompt=f"bad{i}", looks_good=False) for i in range(6)]
        selected = select_calibration_examples(rated)
        assert len(selected) == 2 * MAX_CALIBRATION_PER_VERDICT
        good = [s.prompt for s in selected if s.looks_good]
        bad = [s.prompt for s in selected if not s.looks_good]
        assert good == ["good2", "good3", "good4", "good5"]
        assert bad == ["bad2", "bad3", "bad4", "bad5"]

    def test_preserves_original_order(self):
        rated = [
            _rated(prompt="first", looks_good=True),
            _rated(prompt="second", looks_good=False),
            _rated(prompt="third", looks_good=True),
        ]
        assert [s.prompt for s in select_calibration_examples(rated)] == [
            "first",
            "second",
            "third",
        ]

    def test_skips_empty_and_malformed_entries(self):
        rated = [
            _rated(prompt="", looks_good=True),
            _rated(response="   ", looks_good=True),
            {"prompt": "no verdict", "response": "a"},
            {"prompt": "q", "response": "a", "looks_good": "yes"},
            "not a dict",
            _rated(prompt="kept"),
        ]
        selected = select_calibration_examples(rated)
        assert [s.prompt for s in selected] == ["kept"]


class TestScorerWiring:
    @pytest.mark.asyncio
    async def test_scorer_renders_calibration_into_judge_prompt(self):
        seen: list[str] = []

        async def fake_llm(prompt: str) -> str:
            seen.append(prompt)
            return json.dumps({"consistent": True, "score": 1.0, "reason": "ok"})

        scorer = LlmFaithfulnessScorer(
            llm_call=fake_llm, calibration=[_rated(prompt="human-approved q")]
        )
        await scorer.score(
            pair=GeneratedPair(prompt="q", response="a", source_text="s"), source_text="s"
        )
        assert "<calibration_examples>" in seen[0]
        assert "human-approved q" in seen[0]

    @pytest.mark.asyncio
    async def test_scorer_without_calibration_leaves_prompt_uncalibrated(self):
        seen: list[str] = []

        async def fake_llm(prompt: str) -> str:
            seen.append(prompt)
            return json.dumps({"consistent": True, "score": 1.0, "reason": "ok"})

        scorer = LlmFaithfulnessScorer(llm_call=fake_llm)
        await scorer.score(
            pair=GeneratedPair(prompt="q", response="a", source_text="s"), source_text="s"
        )
        assert "<calibration_examples>" not in seen[0]

    def test_registry_threads_calibration_through(self):
        async def dummy(prompt: str) -> str:
            return "{}"

        settings = SimpleNamespace(datagen_faithfulness_backend="llm")
        scorer = get_faithfulness_scorer(settings, dummy, calibration=[_rated()])
        assert isinstance(scorer, LlmFaithfulnessScorer)
        assert len(scorer.calibration) == 1


class TestBackCompat:
    def test_pairs_input_constructible_without_rated(self):
        payload = GenerateSyntheticPairsInput(
            tenant_id="t", project_id="p", chunks_storage_path="c", task_type="qna"
        )
        assert payload.rated == []

    def test_workflow_rated_param_defaults_to_none(self):
        params = inspect.signature(GenerateDatasetWorkflow.run).parameters
        assert list(params)[-1] == "rated"
        assert params["rated"].default is None
