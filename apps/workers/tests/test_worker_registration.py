"""Startup smoke test for Temporal activity registration.

Regression guard: activities must be registered by their ``@activity.defn``-decorated
bound ``run`` method, not by the holder instance. Registering the instance raises
``TypeError: Activity <unknown> missing attributes`` when ``Worker(...)`` is constructed,
which crashes the worker on startup. This test validates every registered callable the
same way Temporal does, without needing a live Temporal server.
"""

from unittest.mock import MagicMock

from temporalio.activity import _Definition

from src.worker import build_activity_lists


def test_all_registered_activities_are_valid_definitions():
    infra = MagicMock()
    gpu_provider = MagicMock()

    cpu_activities, gpu_activities = build_activity_lists(infra, gpu_provider)

    all_activities = cpu_activities + gpu_activities
    assert len(all_activities) == 19

    names = []
    for callable_ in all_activities:
        # Raises TypeError if the callable is not a valid @activity.defn definition
        # (e.g. an instance was registered instead of its bound `run` method).
        definition = _Definition.must_from_callable(callable_)
        assert definition is not None
        names.append(definition.name)

    # Activity names must be unique across the worker.
    assert len(names) == len(set(names)), f"duplicate activity names: {names}"
    assert "parse_document" in names
