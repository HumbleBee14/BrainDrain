"""Unit tests for iterative-training best-checkpoint selection.

These exercise the pure decision helpers directly (no Temporal server), which
is what the workflow's run() uses to pick the best checkpoint. They guard the
fix for the eval-failure-fallback bug: a failed round must never win "best",
and if no round is successfully evaluated we ship the last trained adapter
rather than an empty path.
"""

import math

from src.workflows.train_iterative import (
    effective_eval_loss,
    is_meaningful_improvement,
    resolve_final_checkpoint,
)


class TestEffectiveEvalLoss:
    def test_successful_eval_returns_finite_value_and_record(self):
        assert effective_eval_loss(0.8, eval_failed=False) == (0.8, 0.8)

    def test_failed_eval_is_ineligible_and_unrecorded(self):
        comparison, recorded = effective_eval_loss(None, eval_failed=True)
        assert comparison == float("inf")
        assert recorded is None

    def test_failed_eval_never_uses_a_supplied_value(self):
        # Even if a (train-loss) value is passed, a failed eval stays +inf/None —
        # the whole point of the fix.
        comparison, recorded = effective_eval_loss(0.001, eval_failed=True)
        assert comparison == float("inf")
        assert recorded is None

    def test_nan_is_ineligible(self):
        comparison, recorded = effective_eval_loss(float("nan"), eval_failed=False)
        assert comparison == float("inf")
        assert recorded is None

    def test_inf_is_ineligible(self):
        assert effective_eval_loss(float("inf"), eval_failed=False) == (float("inf"), None)

    def test_non_numeric_is_ineligible(self):
        assert effective_eval_loss("0.5", eval_failed=False) == (float("inf"), None)
        assert effective_eval_loss(None, eval_failed=False) == (float("inf"), None)

    def test_bool_is_not_treated_as_number(self):
        # bool subclasses int; a True/False eval loss is invalid, not 1.0/0.0.
        assert effective_eval_loss(True, eval_failed=False) == (float("inf"), None)


class TestIsMeaningfulImprovement:
    def test_beats_incumbent_beyond_min_delta(self):
        assert is_meaningful_improvement(0.5, 1.0, 0.1) is True

    def test_improvement_within_min_delta_is_not_meaningful(self):
        assert is_meaningful_improvement(0.95, 1.0, 0.1) is False

    def test_worse_candidate_rejected(self):
        assert is_meaningful_improvement(1.2, 1.0, 0.0) is False

    def test_infinite_candidate_never_improves(self):
        # An ineligible (failed) round can never become best, even vs +inf.
        assert is_meaningful_improvement(float("inf"), float("inf"), 0.0) is False

    def test_first_finite_beats_initial_infinity(self):
        assert is_meaningful_improvement(0.9, float("inf"), 0.01) is True


class TestResolveFinalCheckpoint:
    def test_prefers_selected_best(self):
        assert resolve_final_checkpoint("best", 200, "last", 100) == ("best", 200, None)

    def test_falls_back_to_last_trained_when_no_best(self):
        assert resolve_final_checkpoint("", 0, "last", 100) == (
            "last",
            100,
            "last_trained_adapter",
        )

    def test_returns_none_when_nothing_trained(self):
        assert resolve_final_checkpoint("", 0, "", 0) is None


def test_failed_round_with_lowest_train_loss_is_not_selected():
    """End-to-end of the decision logic: the classic trap the bug fell into.

    Three rounds; the middle one has by far the lowest *train* loss but its eval
    FAILED, and the other two evaluated successfully. Walking the same decisions
    run() makes, the failed round must never become best.
    """
    rounds = [
        {"eval_loss": 1.0, "failed": False, "adapter": "adapter-0", "size": 100},
        {"eval_loss": 0.001, "failed": True, "adapter": "adapter-1", "size": 101},
        {"eval_loss": 1.5, "failed": False, "adapter": "adapter-2", "size": 102},
    ]

    best_path, best_size, best_loss = "", 0, float("inf")
    last_path, last_size = "", 0
    for r in rounds:
        comparison, _recorded = effective_eval_loss(r["eval_loss"], eval_failed=r["failed"])
        last_path, last_size = r["adapter"], r["size"]
        if is_meaningful_improvement(comparison, best_loss, 0.0):
            best_loss, best_path, best_size = comparison, r["adapter"], r["size"]

    resolved = resolve_final_checkpoint(best_path, best_size, last_path, last_size)
    assert resolved == ("adapter-0", 100, None)
    assert best_loss == 1.0
    assert not math.isinf(best_loss)


def test_all_evals_fail_falls_back_to_last_trained():
    rounds = [
        {"eval_loss": None, "failed": True, "adapter": "adapter-0", "size": 100},
        {"eval_loss": None, "failed": True, "adapter": "adapter-1", "size": 101},
    ]

    best_path, best_size, best_loss = "", 0, float("inf")
    last_path, last_size = "", 0
    for r in rounds:
        comparison, _recorded = effective_eval_loss(r["eval_loss"], eval_failed=r["failed"])
        last_path, last_size = r["adapter"], r["size"]
        if is_meaningful_improvement(comparison, best_loss, 0.0):
            best_loss, best_path, best_size = comparison, r["adapter"], r["size"]

    resolved = resolve_final_checkpoint(best_path, best_size, last_path, last_size)
    assert resolved == ("adapter-1", 101, "last_trained_adapter")
