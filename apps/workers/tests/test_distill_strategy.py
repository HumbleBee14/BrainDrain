"""Distill mode: registered as an SFT strategy, never judge-backed."""

from src.activities.train_model import _JUDGE_BACKED_MODES, DistillStrategy, QuickStrategy
from src.activities.training_engine import get_strategy


def test_distill_strategy_registered_as_sft_reuse():
    strategy = get_strategy("distill")
    assert isinstance(strategy, DistillStrategy)
    assert isinstance(strategy, QuickStrategy)
    assert strategy.name == "distill"


def test_distill_is_not_judge_backed():
    # Distill training is a plain SFT pass over teacher-written data; the
    # teacher/judge are used in datagen and evaluation, never during training.
    assert "distill" not in _JUDGE_BACKED_MODES


def test_train_workflow_dispatches_distill_like_quick():
    from pathlib import Path

    source = (Path(__file__).resolve().parents[1] / "src" / "workflows" / "train.py").read_text()
    assert 'mode in ("quick", "distill")' in source
