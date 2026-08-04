"""Tests for the TeacherParitySuite (student-vs-teacher, distill mode only).

The golden holdout's expected answers are teacher outputs by construction, so
parity is judged blind between the student's generation and the stored
teacher answer. The suite must contribute nothing outside distill mode, keep
the overall score untouched (report-only), and propagate judge failures
rather than fabricate numbers.
"""

import pytest

import src.activities.run_evaluation as re_mod
from src.activities.llm_judge import JudgeUnavailableError
from src.activities.run_evaluation import (
    EvaluationContext,
    TeacherParitySuite,
    _compute_overall,
    _suite_pct,
)

DISTILL = EvaluationContext(mode="distill")


def _golden(n: int) -> list[dict]:
    return [
        {
            "messages": [
                {"role": "system", "content": "sys"},
                {"role": "user", "content": f"What is fact {i}?"},
                {"role": "assistant", "content": f"Teacher fact {i}."},
            ]
        }
        for i in range(n)
    ]


class _FakeJudge:
    """Deterministic verdicts: the STUDENT side wins/ties/loses on a cycle."""

    def __init__(self, cycle=("student", "tie", "teacher"), agree: bool = True):
        self.cycle = cycle
        self.agree = agree
        self.compare_calls: list[tuple[str, str, str]] = []

    def compare_ab(self, prompt, response_a, response_b):
        self.compare_calls.append((prompt, response_a, response_b))
        outcome = self.cycle[(len(self.compare_calls) - 1) % len(self.cycle)]
        if outcome == "tie":
            return "tie"
        student_is_a = response_a.startswith("student:")
        if outcome == "student":
            return "A" if student_is_a else "B"
        return "B" if student_is_a else "A"

    def check_correctness(self, answer, expected):
        return self.agree


class _BrokenJudge:
    def compare_ab(self, prompt, response_a, response_b):
        raise JudgeUnavailableError("judge is down")

    def check_correctness(self, answer, expected):
        raise JudgeUnavailableError("judge is down")


def _patch_generation(monkeypatch):
    monkeypatch.setattr(
        re_mod, "_generate", lambda model, tok, prompt, max_new_tokens=512: f"{model}: answer"
    )
    monkeypatch.setattr(re_mod, "_format_prompt", lambda tok, msgs, tools=None: msgs[-1]["content"])


class TestTeacherParitySuite:
    def test_skips_when_not_distill(self, monkeypatch):
        _patch_generation(monkeypatch)
        for context in (None, EvaluationContext(mode=""), EvaluationContext(mode="quick")):
            scores, report = TeacherParitySuite().run(
                "student", None, None, None, _FakeJudge(), [], _golden(3), context=context
            )
            assert scores == {}

    def test_parity_math(self, monkeypatch):
        _patch_generation(monkeypatch)
        judge = _FakeJudge(cycle=("student", "tie", "teacher"))

        scores, report = TeacherParitySuite().run(
            "student", None, None, None, judge, [], _golden(6), context=DISTILL
        )

        # 6 samples on a win/tie/loss cycle: 2 wins, 2 ties, 2 losses.
        assert scores["n"] == 6
        assert scores["win_rate"] == pytest.approx(2 / 6, abs=0.001)
        assert scores["tie_rate"] == pytest.approx(2 / 6, abs=0.001)
        assert scores["parity"] == pytest.approx(4 / 6, abs=0.001)
        assert scores["agreement"] == 1.0
        assert report["num_samples"] == 6

    def test_comparison_is_blind_both_positions_used(self, monkeypatch):
        _patch_generation(monkeypatch)
        judge = _FakeJudge()
        TeacherParitySuite().run(
            "student", None, None, None, judge, [], _golden(20), context=DISTILL
        )
        student_first = sum(1 for _, a, _b in judge.compare_calls if a.startswith("student:"))
        assert 0 < student_first < len(judge.compare_calls)

    def test_judge_failure_propagates_never_fabricates(self, monkeypatch):
        _patch_generation(monkeypatch)
        with pytest.raises(JudgeUnavailableError):
            TeacherParitySuite().run(
                "student", None, None, None, _BrokenJudge(), [], _golden(2), context=DISTILL
            )

    def test_no_golden_set_reports_empty_section(self):
        scores, report = TeacherParitySuite().run(
            "student", None, None, None, _FakeJudge(), [], None, context=DISTILL
        )
        assert scores == {}
        assert "note" in report

    def test_unscorable_samples_yield_no_numbers(self, monkeypatch):
        _patch_generation(monkeypatch)
        bad = [{"messages": [{"role": "user", "content": "q"}, {"role": "tool", "content": "x"}]}]
        scores, report = TeacherParitySuite().run(
            "student", None, None, None, _FakeJudge(), [], bad, context=DISTILL
        )
        assert scores == {}
        assert report["skipped_samples"] == 1

    def test_report_only_never_moves_overall(self):
        suites = [
            type("S", (), {"name": "general", "weight": 0.25})(),
            type("S", (), {"name": "teacher_parity", "weight": 0.0})(),
        ]
        scores = {
            "general": {"finetuned_score": 80},
            "teacher_parity": {"parity": 0.1, "win_rate": 0.0, "tie_rate": 0.1, "n": 10},
        }
        assert _suite_pct("teacher_parity", scores) is None
        assert _compute_overall(scores, suites) == 80.0
