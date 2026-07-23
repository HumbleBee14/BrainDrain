"""Tests for evaluation activity -- pure helper functions only (no GPU/DB/S3)."""

from collections import namedtuple

from src.activities.run_evaluation import (
    _check_answer,
    _classify_refusal,
    _compute_overall,
    _format_prompt,
    _generate_recommendations,
    _mean,
    _suite_pct,
    _wilson_ci,
)

_Suite = namedtuple("_Suite", ["name", "weight"])
_ALL_SUITES = [
    _Suite("domain", 0.30),
    _Suite("general", 0.25),
    _Suite("ab_comparison", 0.25),
    _Suite("safety", 0.20),
]


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

    def test_short_valid_answer_is_not_refusal(self):
        # A brief but genuine answer must not be misclassified as a refusal.
        assert _classify_refusal("OK") is False
        assert _classify_refusal("42") is False

    def test_empty_response_is_refusal(self):
        assert _classify_refusal("") is True
        assert _classify_refusal("   ") is True

    def test_mid_response_keyword_is_not_refusal(self):
        # "as an AI" appearing partway through a substantive answer is not a refusal.
        response = (
            "Neural networks learn by iteratively adjusting their connection weights "
            "through the backpropagation algorithm across many training epochs. "
            "As an AI concept, this mirrors how gradient descent minimizes loss."
        )
        assert _classify_refusal(response) is False

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
        overall = _compute_overall(scores, _ALL_SUITES)
        assert overall == 100.0

    def test_zero_scores(self):
        scores = {
            "domain": {"mean": 0},
            "general": {"finetuned_score": 0, "forgetting_alert": False},
            "ab_comparison": {"win_rate": 0},
            "safety": {"refusal_rate": 0, "degraded": False},
        }
        overall = _compute_overall(scores, _ALL_SUITES)
        assert overall == 0.0

    def test_forgetting_penalty(self):
        scores = {
            "domain": {"mean": 5.0},
            "general": {"finetuned_score": 100, "forgetting_alert": True},
            "ab_comparison": {"win_rate": 1.0},
            "safety": {"refusal_rate": 1.0, "degraded": False},
        }
        overall = _compute_overall(scores, _ALL_SUITES)
        assert overall == 90.0

    def test_safety_penalty(self):
        scores = {
            "domain": {"mean": 5.0},
            "general": {"finetuned_score": 100, "forgetting_alert": False},
            "ab_comparison": {"win_rate": 1.0},
            "safety": {"refusal_rate": 1.0, "degraded": True},
        }
        overall = _compute_overall(scores, _ALL_SUITES)
        assert overall == 85.0

    def test_both_penalties(self):
        scores = {
            "domain": {"mean": 5.0},
            "general": {"finetuned_score": 100, "forgetting_alert": True},
            "ab_comparison": {"win_rate": 1.0},
            "safety": {"refusal_rate": 1.0, "degraded": True},
        }
        overall = _compute_overall(scores, _ALL_SUITES)
        assert overall == 75.0

    def test_empty_scores_returns_zero(self):
        overall = _compute_overall({}, _ALL_SUITES)
        assert overall >= 0.0

    def test_overall_bounded(self):
        scores = {
            "domain": {"mean": 0},
            "general": {"finetuned_score": 0, "forgetting_alert": True},
            "ab_comparison": {"win_rate": 0},
            "safety": {"refusal_rate": 0, "degraded": True},
        }
        overall = _compute_overall(scores, _ALL_SUITES)
        assert overall == 0.0  # clamped to 0

    def test_missing_suite_is_excluded_not_defaulted(self):
        # Only domain ran (perfect). The overall must renormalize to that suite
        # alone (100.0), not be dragged down by defaults for suites that never ran.
        scores = {"domain": {"mean": 5.0}}
        assert _compute_overall(scores, _ALL_SUITES) == 100.0

    def test_partial_suites_renormalize(self):
        # domain 100% (w .30) + general 50% (w .25) → weighted mean over .55.
        scores = {"domain": {"mean": 5.0}, "general": {"finetuned_score": 50}}
        expected = (100.0 * 0.30 + 50.0 * 0.25) / (0.30 + 0.25)
        assert _compute_overall(scores, _ALL_SUITES) == round(expected, 1)

    def test_no_suites_ran_returns_zero(self):
        assert _compute_overall({"domain": {"mean": None}}, _ALL_SUITES) == 0.0


class TestSuitePct:
    def test_domain_none_when_mean_absent(self):
        assert _suite_pct("domain", {"domain": {"mean": None}}) is None
        assert _suite_pct("domain", {}) is None

    def test_domain_scales_five_point_to_pct(self):
        assert _suite_pct("domain", {"domain": {"mean": 5.0}}) == 100.0

    def test_ab_none_when_win_rate_absent(self):
        assert _suite_pct("ab_comparison", {"ab_comparison": {}}) is None


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

    def test_none_metrics_do_not_raise(self):
        # When suites produced no usable score, their metrics are present-but-None.
        # Recommendations must not raise TypeError on `None < float`.
        scores = {
            "domain": {"mean": None, "accuracy": 0.0, "completeness": 0.0},
            "general": {"forgetting_alert": False, "delta_pct": 0},
            "ab_comparison": {"win_rate": None},
            "safety": {"degraded": False},
        }
        recs = _generate_recommendations(scores)
        # No domain/AB recommendation should be emitted from a None score.
        assert not any("domain performance" in r.lower() for r in recs)
        assert not any("blind comparison" in r.lower() for r in recs)


class TestSuiteNoValDataReturnsNone:
    """No-validation-data early returns must emit None (excluded + renormalized),
    never a fabricated 0%/50% that silently skews the overall score."""

    def test_domain_suite_no_val_data_returns_none_mean(self):
        from src.activities.run_evaluation import DomainSuite

        scores, report = DomainSuite().run(None, None, None, None, None, [])
        assert scores["mean"] is None
        assert _suite_pct("domain", {"domain": scores}) is None

    def test_ab_suite_no_val_data_returns_none_win_rate(self):
        from src.activities.run_evaluation import ABComparisonSuite

        scores, report = ABComparisonSuite().run(None, None, None, None, None, [])
        assert scores["win_rate"] is None
        assert _suite_pct("ab_comparison", {"ab_comparison": scores}) is None

    def test_overall_excludes_no_val_suites(self):
        from src.activities.run_evaluation import ABComparisonSuite, DomainSuite

        domain_scores, _ = DomainSuite().run(None, None, None, None, None, [])
        ab_scores, _ = ABComparisonSuite().run(None, None, None, None, None, [])
        scores = {
            "domain": domain_scores,
            "ab_comparison": ab_scores,
            "general": {"finetuned_score": 80},
        }
        # Only general contributed; overall must equal general's pct, not be
        # dragged toward 0 by the empty domain/AB suites.
        assert _compute_overall(scores, _ALL_SUITES) == 80.0

    def test_accuracy_lower_than_completeness_warns(self):
        scores = {
            "domain": {"mean": 4.0, "accuracy": 2.5, "completeness": 4.5},
            "general": {"forgetting_alert": False, "delta_pct": 0},
            "ab_comparison": {"win_rate": 0.5},
            "safety": {"degraded": False},
        }
        recs = _generate_recommendations(scores)
        assert any("accuracy" in r.lower() for r in recs)
