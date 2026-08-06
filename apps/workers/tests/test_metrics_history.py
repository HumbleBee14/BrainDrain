"""Tests for the durable loss-history buffer.

The live Redis stream is capped and lost on page reload, so every logged loss
point is also buffered per-job and folded into the persisted job metrics at
completion. Draining must downsample long runs, always keep the final point,
and leave nothing behind for the job id.
"""

from src.activities.train_model import (
    _HISTORY_MAX_POINTS,
    _METRICS_HISTORY,
    _drain_history,
    _record_history_point,
    _stitch_iteration_history,
)


def _point(step: int, phase: str = "sft") -> dict:
    return {"step": step, "epoch": 0.5, "loss": 1.0 / (step + 1), "phase": phase}


class TestDrainHistory:
    def test_short_run_returned_verbatim(self):
        for step in range(10):
            _record_history_point("job-short", _point(step))
        history = _drain_history("job-short")
        assert [p["step"] for p in history] == list(range(10))

    def test_long_run_downsampled_and_keeps_last_point(self):
        total = _HISTORY_MAX_POINTS * 4
        for step in range(total):
            _record_history_point("job-long", _point(step))
        history = _drain_history("job-long")
        assert len(history) == _HISTORY_MAX_POINTS
        assert history[0]["step"] == 0
        assert history[-1]["step"] == total - 1
        steps = [p["step"] for p in history]
        assert steps == sorted(steps)

    def test_drain_removes_buffer(self):
        _record_history_point("job-drained", _point(1))
        _drain_history("job-drained")
        assert "job-drained" not in _METRICS_HISTORY
        assert _drain_history("job-drained") == []

    def test_phases_preserved_in_order(self):
        for step in range(3):
            _record_history_point("job-phases", _point(step, phase="sft"))
        for step in range(3, 6):
            _record_history_point("job-phases", _point(step, phase="dpo"))
        history = _drain_history("job-phases")
        assert [p["phase"] for p in history] == ["sft"] * 3 + ["dpo"] * 3

    def test_jobs_are_isolated(self):
        _record_history_point("job-a", _point(1))
        _record_history_point("job-b", _point(2))
        assert [p["step"] for p in _drain_history("job-a")] == [1]
        assert [p["step"] for p in _drain_history("job-b")] == [2]


class TestStitchIterationHistory:
    def test_rounds_concatenate_with_cumulative_steps(self):
        metrics = {
            "iter_0": {"loss_history": [_point(1, "iter_0"), _point(5, "iter_0")]},
            "iter_1": {"loss_history": [_point(1, "iter_1"), _point(5, "iter_1")]},
            "iter_0_eval_loss": 0.4,
        }
        history = _stitch_iteration_history(metrics)
        assert [p["step"] for p in history] == [1, 5, 6, 10]
        assert [p["phase"] for p in history] == ["iter_0", "iter_0", "iter_1", "iter_1"]

    def test_rounds_ordered_numerically_not_lexically(self):
        metrics = {
            f"iter_{n}": {"loss_history": [_point(1, f"iter_{n}")]} for n in [10, 2, 0]
        }
        history = _stitch_iteration_history(metrics)
        assert [p["phase"] for p in history] == ["iter_0", "iter_2", "iter_10"]

    def test_per_round_histories_removed_after_stitch(self):
        metrics = {"iter_0": {"loss_history": [_point(1)], "iter_0_train_loss": 0.5}}
        _stitch_iteration_history(metrics)
        assert "loss_history" not in metrics["iter_0"]

    def test_no_round_histories_returns_empty(self):
        assert _stitch_iteration_history({"iter_0": {"sft_train_loss": 0.5}}) == []

    def test_stitched_series_downsampled(self):
        metrics = {
            "iter_0": {
                "loss_history": [_point(s, "iter_0") for s in range(_HISTORY_MAX_POINTS)]
            },
            "iter_1": {
                "loss_history": [_point(s, "iter_1") for s in range(_HISTORY_MAX_POINTS)]
            },
        }
        history = _stitch_iteration_history(metrics)
        assert len(history) == _HISTORY_MAX_POINTS
