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
