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
