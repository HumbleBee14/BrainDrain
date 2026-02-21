"""Tests for evaluation activity -- pure helper functions only (no GPU/DB/S3)."""
import math

import pytest

from src.activities.run_evaluation import (
    _check_answer,
    _classify_refusal,
    _compute_overall,
    _format_prompt,
    _generate_recommendations,
    _mean,
    _wilson_ci,
)


class TestClassifyRefusal:
    """Test keyword-based refusal classification."""

    def test_clear_refusal(self):
        assert _classify_refusal("I cannot help with that request.") is True

    def test_ai_identity_refusal(self):
        assert _classify_refusal("As an AI, I must decline this request.") is True

    def test_apology_refusal(self):
        assert _classify_refusal("I apologize, but I can't assist with this.") is True

    def test_normal_response_not_refusal(self):
        response = "The capital of France is Paris. It is located in the north of the country."
        assert _classify_refusal(response) is False

    def test_very_short_response_is_refusal(self):
        assert _classify_refusal("OK") is True

    def test_empty_response_is_refusal(self):
        assert _classify_refusal("") is True

    def test_long_normal_response(self):
        response = "Here is a detailed explanation of how machine learning works. " * 5
        assert _classify_refusal(response) is False


class TestMean:
    def test_normal_list(self):
        assert _mean([1, 2, 3, 4, 5]) == 3.0

    def test_empty_list(self):
        assert _mean([]) == 0.0

    def test_single_element(self):
        assert _mean([7.5]) == 7.5

    def test_result_is_rounded(self):
        result = _mean([1, 2, 3])
        assert result == 2.0


class TestWilsonCI:
    def test_zero_total_returns_full_interval(self):
        low, high = _wilson_ci(0, 0)
        assert low == 0.0
        assert high == 1.0

    def test_all_successes(self):
        low, high = _wilson_ci(100, 100)
        assert low > 0.9
        assert high <= 1.0

    def test_no_successes(self):
        low, high = _wilson_ci(0, 100)
        assert low >= 0.0
        assert high < 0.1

    def test_half_successes(self):
        low, high = _wilson_ci(50, 100)
        assert 0.35 < low < 0.5
        assert 0.5 < high < 0.65

    def test_bounds_are_valid(self):
        low, high = _wilson_ci(3, 10)
        assert 0.0 <= low <= high <= 1.0


class TestFormatPrompt:
    def test_single_user_message(self):
        messages = [{"role": "user", "content": "Hello"}]
        result = _format_prompt(messages)
        assert "<|im_start|>user" in result
        assert "Hello" in result
        assert "<|im_start|>assistant" in result

    def test_system_and_user_messages(self):
        messages = [
            {"role": "system", "content": "You are helpful."},
            {"role": "user", "content": "Hi"},
        ]
        result = _format_prompt(messages)
        assert "<|im_start|>system" in result
        assert "You are helpful." in result
        assert "<|im_start|>user" in result
        assert result.endswith("<|im_start|>assistant\n")

    def test_empty_messages(self):
        result = _format_prompt([])
        assert "<|im_start|>assistant" in result


class TestCheckAnswer:
    """Test answer checking (exact match mode only -- no LLM judge needed)."""

    def test_exact_match_correct(self):
        assert _check_answer("Paris", "paris", "exact_match", None) is True

    def test_exact_match_contained(self):
        assert _check_answer("The answer is Paris", "paris", "exact_match", None) is True

    def test_exact_match_wrong(self):
        assert _check_answer("London", "Paris", "exact_match", None) is False


class TestComputeOverall:
    def test_perfect_scores(self):
        scores = {
            "domain": {"mean": 5.0},
            "general": {"finetuned_score": 100, "forgetting_alert": False},
            "ab_comparison": {"win_rate": 1.0},
            "safety": {"refusal_rate": 1.0, "degraded": False},
        }
        overall = _compute_overall(scores, [])
        assert overall == 100.0

    def test_zero_scores(self):
        scores = {
            "domain": {"mean": 0},
            "general": {"finetuned_score": 0, "forgetting_alert": False},
            "ab_comparison": {"win_rate": 0},
            "safety": {"refusal_rate": 0, "degraded": False},
        }
        overall = _compute_overall(scores, [])
        assert overall == 0.0

    def test_forgetting_penalty(self):
        scores = {
            "domain": {"mean": 5.0},
            "general": {"finetuned_score": 100, "forgetting_alert": True},
            "ab_comparison": {"win_rate": 1.0},
            "safety": {"refusal_rate": 1.0, "degraded": False},
        }
        overall = _compute_overall(scores, [])
        assert overall == 90.0

    def test_safety_penalty(self):
        scores = {
            "domain": {"mean": 5.0},
            "general": {"finetuned_score": 100, "forgetting_alert": False},
            "ab_comparison": {"win_rate": 1.0},
            "safety": {"refusal_rate": 1.0, "degraded": True},
        }
        overall = _compute_overall(scores, [])
        assert overall == 85.0

    def test_both_penalties(self):
        scores = {
            "domain": {"mean": 5.0},
            "general": {"finetuned_score": 100, "forgetting_alert": True},
            "ab_comparison": {"win_rate": 1.0},
            "safety": {"refusal_rate": 1.0, "degraded": True},
        }
        overall = _compute_overall(scores, [])
        assert overall == 75.0

    def test_empty_scores_returns_zero(self):
        overall = _compute_overall({}, [])
        assert overall >= 0.0

    def test_overall_bounded(self):
        scores = {
            "domain": {"mean": 0},
            "general": {"finetuned_score": 0, "forgetting_alert": True},
            "ab_comparison": {"win_rate": 0},
            "safety": {"refusal_rate": 0, "degraded": True},
        }
        overall = _compute_overall(scores, [])
        assert overall == 0.0  # clamped to 0


class TestGenerateRecommendations:
    def test_good_scores_get_ready_message(self):
        scores = {
            "domain": {"mean": 4.5, "accuracy": 4.5, "completeness": 4.0},
            "general": {"forgetting_alert": False, "delta_pct": 2.0},
            "ab_comparison": {"win_rate": 0.65},
            "safety": {"degraded": False},
        }
        recs = _generate_recommendations(scores)
        assert any("ready" in r.lower() or "good" in r.lower() for r in recs)

    def test_low_domain_score_warns(self):
        scores = {
            "domain": {"mean": 2.0, "accuracy": 2.0, "completeness": 3.0},
            "general": {"forgetting_alert": False, "delta_pct": 0},
            "ab_comparison": {"win_rate": 0.5},
            "safety": {"degraded": False},
        }
        recs = _generate_recommendations(scores)
        assert any("domain" in r.lower() for r in recs)

    def test_forgetting_alert_warns(self):
        scores = {
            "domain": {"mean": 4.0, "accuracy": 4.0, "completeness": 4.0},
            "general": {"forgetting_alert": True, "delta_pct": -12},
            "ab_comparison": {"win_rate": 0.5},
            "safety": {"degraded": False},
        }
        recs = _generate_recommendations(scores)
        assert any("forgetting" in r.lower() for r in recs)

    def test_safety_degradation_warns(self):
        scores = {
            "domain": {"mean": 4.0, "accuracy": 4.0, "completeness": 4.0},
            "general": {"forgetting_alert": False, "delta_pct": 0},
            "ab_comparison": {"win_rate": 0.5},
            "safety": {"degraded": True},
        }
        recs = _generate_recommendations(scores)
        assert any("safety" in r.lower() for r in recs)

    def test_low_win_rate_warns(self):
        scores = {
            "domain": {"mean": 4.0, "accuracy": 4.0, "completeness": 4.0},
            "general": {"forgetting_alert": False, "delta_pct": 0},
            "ab_comparison": {"win_rate": 0.3},
            "safety": {"degraded": False},
        }
        recs = _generate_recommendations(scores)
        assert any("base model outperforms" in r.lower() for r in recs)

    def test_high_win_rate_praises(self):
        scores = {
            "domain": {"mean": 4.5, "accuracy": 4.5, "completeness": 4.5},
            "general": {"forgetting_alert": False, "delta_pct": 5},
            "ab_comparison": {"win_rate": 0.8},
            "safety": {"degraded": False},
        }
        recs = _generate_recommendations(scores)
        assert any("strong" in r.lower() or "effective" in r.lower() for r in recs)

    def test_accuracy_lower_than_completeness_warns(self):
        scores = {
            "domain": {"mean": 4.0, "accuracy": 2.5, "completeness": 4.5},
            "general": {"forgetting_alert": False, "delta_pct": 0},
            "ab_comparison": {"win_rate": 0.5},
            "safety": {"degraded": False},
        }
        recs = _generate_recommendations(scores)
        assert any("accuracy" in r.lower() for r in recs)
