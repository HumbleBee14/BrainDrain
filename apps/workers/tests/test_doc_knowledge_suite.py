"""Tests for the DocumentKnowledgeSuite (golden-holdout evaluation).

The golden set holds pairs generated from chunks the model never trained on;
the suite reports the fine-tuned model's judged quality on them and the
*knowledge lift* over the base model. A missing/unscorable golden set must be
excluded from the overall score (None), never counted as a fabricated zero.
"""

from collections import namedtuple

import src.activities.run_evaluation as re_mod
from src.activities.run_evaluation import (
    DocumentKnowledgeSuite,
    _compute_overall,
    _generate_recommendations,
    _suite_pct,
)

_Suite = namedtuple("_Suite", ["name", "weight"])


def _golden(n: int) -> list[dict]:
    return [
        {
            "messages": [
                {"role": "system", "content": "sys"},
                {"role": "user", "content": f"What is fact {i}?"},
                {"role": "assistant", "content": f"Fact {i} is X."},
            ]
        }
        for i in range(n)
    ]


class _FakeJudge:
    """Scores the fine-tuned answers high and base answers low."""

    def __init__(self, ft_score=5.0, base_score=2.0, none_for=()):
        self.ft_score = ft_score
        self.base_score = base_score
        self.none_for = set(none_for)
        self.calls = 0

    def score_domain(self, prompt, generated, expected):
        self.calls += 1
        if generated in self.none_for:
            return {"accuracy": None, "completeness": None, "faithfulness": None}
        score = self.ft_score if generated.startswith("ft:") else self.base_score
        return {"accuracy": score, "completeness": score, "faithfulness": score}


def _patch_generation(monkeypatch):
    """Route generation through markers so the fake judge can tell FT from base."""
    models_seen = []

    def fake_generate(model, tokenizer, prompt, max_new_tokens=512):
        models_seen.append(model)
        return f"{model}: answer"

    monkeypatch.setattr(re_mod, "_generate", fake_generate)
    monkeypatch.setattr(re_mod, "_format_prompt", lambda tok, msgs: msgs[-1]["content"])
    return models_seen


class TestDocumentKnowledgeSuite:
    def test_no_golden_set_returns_none_and_is_excluded(self):
        scores, report = DocumentKnowledgeSuite().run(None, None, None, None, None, [], None)
        assert scores["mean"] is None
        assert scores["knowledge_lift"] is None
        assert _suite_pct("doc_knowledge", {"doc_knowledge": scores}) is None

    def test_positive_knowledge_lift(self, monkeypatch):
        _patch_generation(monkeypatch)
        judge = _FakeJudge(ft_score=5.0, base_score=2.0)

        scores, report = DocumentKnowledgeSuite().run(
            "ft", "tok_ft", "base", "tok_base", judge, [], golden_dataset=_golden(4)
        )

        assert scores["mean"] == 5.0
        assert scores["base_mean"] == 2.0
        assert scores["knowledge_lift"] == 3.0
        assert scores["num_samples"] == 4
        assert report["num_samples"] == 4
        # Both models were judged for every sample.
        assert judge.calls == 8

    def test_unscorable_sample_skipped_on_both_sides(self, monkeypatch):
        _patch_generation(monkeypatch)
        # The base answer can't be scored -> the sample must not count for FT either.
        judge = _FakeJudge(none_for={"base: answer"})

        scores, _ = DocumentKnowledgeSuite().run(
            "ft", "tok_ft", "base", "tok_base", judge, [], golden_dataset=_golden(3)
        )

        assert scores["mean"] is None
        assert scores["knowledge_lift"] is None

    def test_pct_maps_five_point_mean_to_percent(self):
        assert _suite_pct("doc_knowledge", {"doc_knowledge": {"mean": 4.0}}) == 80.0

    def test_overall_includes_doc_knowledge_weight(self):
        suites = [_Suite("general", 0.25), _Suite("doc_knowledge", 0.30)]
        scores = {
            "general": {"finetuned_score": 80},
            "doc_knowledge": {"mean": 5.0},  # 100%
        }
        # (80*0.25 + 100*0.30) / 0.55 ≈ 90.9
        assert _compute_overall(scores, suites) == 90.9

    def test_missing_golden_renormalizes_overall(self):
        suites = [_Suite("general", 0.25), _Suite("doc_knowledge", 0.30)]
        scores = {
            "general": {"finetuned_score": 80},
            "doc_knowledge": {"mean": None, "knowledge_lift": None},
        }
        assert _compute_overall(scores, suites) == 80.0


class TestKnowledgeLiftRecommendation:
    def test_zero_lift_warns(self):
        scores = {"doc_knowledge": {"mean": 3.0, "knowledge_lift": 0.0}}
        recs = _generate_recommendations(scores)
        assert any("document knowledge" in r.lower() for r in recs)

    def test_positive_lift_does_not_warn(self):
        scores = {"doc_knowledge": {"mean": 4.0, "knowledge_lift": 1.5}}
        recs = _generate_recommendations(scores)
        assert not any("document knowledge" in r.lower() for r in recs)

    def test_none_lift_does_not_crash_or_warn(self):
        scores = {"doc_knowledge": {"mean": None, "knowledge_lift": None}}
        recs = _generate_recommendations(scores)
        assert not any("document knowledge" in r.lower() for r in recs)
