"""The message shown to a user must be the cause, not Temporal's wrapper."""

from temporalio.exceptions import ActivityError, ApplicationError

from src.failure_message import root_cause_message


def _wrapped(message: str) -> ActivityError:
    """Rebuild the shape a workflow actually catches: wrapper around cause."""
    cause = ApplicationError(message, non_retryable=True)
    error = ActivityError(
        "Activity task failed",
        scheduled_event_id=1,
        started_event_id=2,
        identity="test",
        activity_type="generate_synthetic_pairs",
        activity_id="1",
        retry_state=None,
    )
    error.__cause__ = cause
    return error


def test_unwraps_to_the_actionable_message():
    err = _wrapped("No LLM API key configured. Add your provider key under Settings -> LLM.")
    assert root_cause_message(err) == (
        "No LLM API key configured. Add your provider key under Settings -> LLM."
    )


def test_wrapper_alone_falls_back():
    err = _wrapped("")
    assert root_cause_message(err) == "Generation failed"


def test_plain_exception_is_returned_as_is():
    assert root_cause_message(RuntimeError("disk full")) == "disk full"


def test_walks_multiple_layers():
    outer = ActivityError(
        "Activity task failed",
        scheduled_event_id=1,
        started_event_id=2,
        identity="test",
        activity_type="a",
        activity_id="1",
        retry_state=None,
    )
    middle = RuntimeError("wrapping layer")
    middle.__cause__ = ValueError("the real problem")
    outer.__cause__ = middle
    assert root_cause_message(outer) == "the real problem"


def test_terminates_on_a_self_referential_chain():
    err = RuntimeError("loop")
    err.__cause__ = err
    assert root_cause_message(err) == "loop"
