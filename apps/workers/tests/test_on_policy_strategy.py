"""Dispatch and engine selection for the on-policy strategy.

The composite registry key keeps the internal strategy name out of the public
`TrainingMode`, exactly as the logit strategy does. What is new here is that a
strategy can require a training engine: on-policy cannot run under Unsloth,
because the image that hosts its teacher cannot contain Unsloth at all.
"""

import pytest

from src.activities.train_model import DistillOnPolicyStrategy, resolve_strategy_key
from src.activities.training_engine import (
    TransformersEngine,
    UnslothEngine,
    get_engine,
    get_strategy,
)


class Settings:
    def __init__(self, engine="unsloth"):
        self.training_engine = engine


@pytest.mark.parametrize(
    ("mode", "method", "expected"),
    [
        ("distill", "on_policy", "distill_on_policy"),
        ("distill", "logit", "distill_logit"),
        ("distill", "text", "distill"),
        ("distill", None, "distill"),
    ],
)
def test_on_policy_joins_the_composite_key_without_new_modes(mode, method, expected):
    assert resolve_strategy_key(mode, method) == expected


def test_on_policy_is_not_reachable_from_a_non_distill_mode():
    """A method with no meaning for a mode is an error, not something to ignore:
    running plain SFT for a job that booked a teacher would report success for
    training that never happened."""
    with pytest.raises(ValueError, match="no meaning for mode"):
        resolve_strategy_key("quick", "on_policy")


def test_an_unknown_method_lists_what_exists():
    with pytest.raises(ValueError, match="on_policy"):
        resolve_strategy_key("distill", "telepathy")


def test_the_strategy_is_registered_under_its_key():
    assert isinstance(get_strategy("distill_on_policy"), DistillOnPolicyStrategy)


def test_the_strategy_requires_the_transformers_engine():
    assert DistillOnPolicyStrategy.required_engine == "transformers"


def test_a_required_engine_overrides_configuration():
    """The image that can serve a teacher has no Unsloth in it, so a run must not
    depend on an env var being set correctly in that image."""
    engine = get_engine(Settings(engine="unsloth"), required="transformers")

    assert isinstance(engine, TransformersEngine)


def test_configuration_still_decides_when_nothing_is_required():
    assert isinstance(get_engine(Settings(engine="unsloth")), UnslothEngine)


def test_an_unknown_required_engine_fails_loudly():
    with pytest.raises(ValueError, match="Unknown training_engine"):
        get_engine(Settings(), required="jax_someday")


def test_other_strategies_declare_no_engine_and_keep_the_faster_path():
    """Only the strategy that needs a live teacher pays the Unsloth-less cost."""
    for key in ("quick", "distill", "distill_logit", "aligned", "reasoning"):
        assert getattr(get_strategy(key), "required_engine", None) is None


def test_only_on_policy_declares_a_resident_teacher():
    """The declaration is what makes the caller carve up the container's GPUs, and
    doing that to a single-card run would refuse a job that was fine."""
    assert DistillOnPolicyStrategy.runs_resident_teacher is True
    for key in ("quick", "distill", "distill_logit", "aligned", "reasoning"):
        assert getattr(get_strategy(key), "runs_resident_teacher", False) is False


@pytest.mark.asyncio
async def test_the_student_gpu_is_claimed_before_any_weights_are_loaded(monkeypatch):
    """The ordering IS the fix. `CUDA_VISIBLE_DEVICES` is inert once a process has
    touched CUDA, so a reservation made after `load_model` leaves the student on
    device 0 — the card the teacher fills to 90%. This test fails if the two calls
    are ever swapped back.
    """
    import src.activities.train_model as tm

    from .helpers_training_core import fake_core_dependencies, run_core

    order = fake_core_dependencies(monkeypatch, strategy=_TeacherStrategy())
    monkeypatch.setattr(
        tm,
        "_reserve_student_devices",
        lambda strategy: order.append("reserve") or (0,),
    )

    await run_core(mode="distill", hyperparams={"distill_method": "on_policy"})

    assert order.index("reserve") < order.index("load_model")


@pytest.mark.asyncio
async def test_the_reserved_teacher_gpu_reaches_the_strategy(monkeypatch):
    """Whoever claims the cards has to say which ones, or the teacher falls back to
    a default and lands on the student's."""
    import src.activities.train_model as tm

    from .helpers_training_core import fake_core_dependencies, run_core

    strategy = _TeacherStrategy()
    fake_core_dependencies(monkeypatch, strategy=strategy)
    monkeypatch.setattr(tm, "_reserve_student_devices", lambda _: (2, 3))

    await run_core(mode="distill", hyperparams={"distill_method": "on_policy"})

    assert strategy.seen["teacher_devices"] == (2, 3)


class _TeacherStrategy:
    name = "distill_on_policy"
    required_engine = "transformers"
    runs_resident_teacher = True

    def __init__(self):
        self.seen = {}

    def execute(self, **kwargs):
        self.seen = kwargs
        return {"train_runtime": 1.0}
