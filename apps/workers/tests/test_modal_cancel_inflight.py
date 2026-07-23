"""Tests for in-flight Modal cancellation when a training activity is cancelled.

When the user cancels a job, the Rust control plane now asks Temporal to
*cancel* (not terminate) the workflow, so the running activity receives an
`asyncio.CancelledError`. The Modal poll loop in `_run_remote` must catch that,
cancel the remote `FunctionCall` so the GPU stops billing immediately, and then
re-raise so Temporal still sees the activity as cancelled. The periodic orphan
sweep remains the backstop if this in-line cancel cannot be sent.
"""

import asyncio
import sys
import types

import pytest

from src.gpu_provider import ModalGpuProvider


class _CancelSpy:
    """Stand-in for FunctionCall.cancel exposing the async `.aio` variant."""

    def __init__(self, record, *, raises=False):
        self._record = record
        self._raises = raises

    async def aio(self):
        if self._raises:
            raise RuntimeError("modal unavailable")
        self._record.append("cancelled")


class _FakeFC:
    """A Modal FunctionCall that never finishes (always times out)."""

    def __init__(self, record, *, cancel_raises=False):
        self.object_id = "fc-inflight"
        self.cancel = _CancelSpy(record, raises=cancel_raises)

        async def _get(timeout=0):
            raise TimeoutError  # never ready → poll loop keeps waiting

        self.get = types.SimpleNamespace(aio=_get)


def _install_fake_modal(monkeypatch, fc):
    async def _spawn(payload):
        return fc

    fn = types.SimpleNamespace(
        with_options=lambda **kw: types.SimpleNamespace(spawn=types.SimpleNamespace(aio=_spawn))
    )
    fake_modal = types.ModuleType("modal")
    fake_modal.Function = types.SimpleNamespace(from_name=lambda *a, **k: fn)
    fake_modal.FunctionCall = types.SimpleNamespace(from_id=lambda cid: fc)
    monkeypatch.setitem(sys.modules, "modal", fake_modal)


class _FakeDB:
    def __init__(self):
        self.executed = []

    async def fetchval(self, query, *args):
        return None  # no stored reservation → spawn fresh

    async def execute(self, query, *args):
        self.executed.append((query, args))


def _provider(monkeypatch, fc):
    _install_fake_modal(monkeypatch, fc)
    settings = types.SimpleNamespace(
        modal_poll_interval_secs=15,
        modal_app_name="platform-training",
        modal_function_name="train",
    )
    infra = types.SimpleNamespace(db=_FakeDB(), settings=settings)
    return ModalGpuProvider(infra)


def _patch_activity_and_sleep(monkeypatch, *, cancel_on_sleep=True):
    """Heartbeat is a no-op; the first `asyncio.sleep` raises CancelledError,
    simulating Temporal delivering cancellation while the activity waits."""
    import temporalio.activity as activity_mod

    monkeypatch.setattr(activity_mod, "heartbeat", lambda *a, **k: None, raising=False)

    async def _sleep(_secs):
        if cancel_on_sleep:
            raise asyncio.CancelledError
        return None

    monkeypatch.setattr(asyncio, "sleep", _sleep)


async def _invoke(provider):
    return await provider._run_remote(
        function_name="train",
        payload={"input": {}},
        gpu="T4",
        table="training_jobs",
        row_id="00000000-0000-0000-0000-000000000001",
        tenant_id="00000000-0000-0000-0000-000000000002",
        label="training",
        clear_after=False,
    )


@pytest.mark.asyncio
async def test_cancellation_cancels_inflight_modal_call(monkeypatch):
    record = []
    fc = _FakeFC(record)
    provider = _provider(monkeypatch, fc)
    _patch_activity_and_sleep(monkeypatch)

    with pytest.raises(asyncio.CancelledError):
        await _invoke(provider)

    # The in-flight Modal call was cancelled before the activity unwound.
    assert record == ["cancelled"]


@pytest.mark.asyncio
async def test_cancellation_still_propagates_when_modal_cancel_fails(monkeypatch):
    """A failed Modal cancel must NOT swallow the cancellation — the activity
    still cancels (orphan sweep is the backstop)."""
    record = []
    fc = _FakeFC(record, cancel_raises=True)
    provider = _provider(monkeypatch, fc)
    _patch_activity_and_sleep(monkeypatch)

    with pytest.raises(asyncio.CancelledError):
        await _invoke(provider)

    # cancel was attempted (raised internally) but never recorded success.
    assert record == []
