"""Tests for cancelling Modal GPU calls orphaned by cancelled/reaped jobs.

A cancelled (workflow terminated) or reaped (stuck worker) job leaves its
Modal reservation in place while the remote GPU keeps running and billing.
The reconciliation sweep must cancel exactly those calls, clear the
reservation, and never touch actively-running jobs.
"""

import sys
import types

import pytest

from src.gpu_provider import _extract_call_id, cancel_orphaned_gpu_calls


class TestExtractCallId:
    def test_tagged_value(self):
        assert _extract_call_id("train:fc-123") == "fc-123"

    def test_tagged_value_with_colon_in_id(self):
        # partition splits on the FIRST colon only.
        assert _extract_call_id("run_evaluation:fc:weird") == "fc:weird"

    def test_bare_legacy_value(self):
        assert _extract_call_id("fc-legacy") == "fc-legacy"

    def test_empty_and_none(self):
        assert _extract_call_id("") is None
        assert _extract_call_id(None) is None


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
    def __init__(self, call_id, record, fail_ids):
        self.call_id = call_id
        self.cancel = _CancelSpy(record, raises=call_id in fail_ids)


def _install_fake_modal(monkeypatch, record, fail_ids=()):
    cancelled_ids = []

    def from_id(call_id):
        cancelled_ids.append(call_id)
        return _FakeFC(call_id, record, set(fail_ids))

    fake_modal = types.ModuleType("modal")
    fake_modal.FunctionCall = types.SimpleNamespace(from_id=from_id)
    monkeypatch.setitem(sys.modules, "modal", fake_modal)
    return cancelled_ids


class _FakeDB:
    def __init__(self, rows):
        self._rows = rows
        self.executed = []

    async def fetch(self, query, *args):
        return self._rows

    async def execute(self, query, *args):
        self.executed.append((query, args))


def _infra(rows):
    return types.SimpleNamespace(db=_FakeDB(rows))


@pytest.mark.asyncio
async def test_cancels_and_clears_each_orphan(monkeypatch):
    record = []
    cancelled_ids = _install_fake_modal(monkeypatch, record)
    rows = [
        {"id": "j1", "tenant_id": "t1", "modal_call_id": "train:fc-a", "tbl": "training_jobs"},
        {"id": "e1", "tenant_id": "t1", "modal_call_id": "fc-legacy", "tbl": "evaluations"},
    ]
    infra = _infra(rows)

    n = await cancel_orphaned_gpu_calls(infra)

    assert n == 2
    # The bare call ids (not the tags) are what gets cancelled.
    assert cancelled_ids == ["fc-a", "fc-legacy"]
    assert record == ["cancelled", "cancelled"]
    # Each row's reservation is cleared, routed to the correct table's SQL.
    assert len(infra.db.executed) == 2
    assert "training_jobs" in infra.db.executed[0][0]
    assert infra.db.executed[0][1] == ("j1", "t1")
    assert "evaluations" in infra.db.executed[1][0]


@pytest.mark.asyncio
async def test_empty_reservation_cleared_without_cancelling(monkeypatch):
    record = []
    cancelled_ids = _install_fake_modal(monkeypatch, record)
    rows = [{"id": "j1", "tenant_id": "t1", "modal_call_id": "", "tbl": "training_jobs"}]
    infra = _infra(rows)

    n = await cancel_orphaned_gpu_calls(infra)

    assert n == 0
    assert cancelled_ids == []  # nothing to cancel
    assert len(infra.db.executed) == 1  # but the empty reservation is still cleared


@pytest.mark.asyncio
async def test_failed_cancel_leaves_reservation_for_retry(monkeypatch):
    record = []
    _install_fake_modal(monkeypatch, record, fail_ids=["fc-bad"])
    rows = [
        {"id": "j1", "tenant_id": "t1", "modal_call_id": "train:fc-bad", "tbl": "training_jobs"},
        {"id": "j2", "tenant_id": "t1", "modal_call_id": "train:fc-ok", "tbl": "training_jobs"},
    ]
    infra = _infra(rows)

    n = await cancel_orphaned_gpu_calls(infra)

    # Only the good one is counted and cleared; the failed one is left for the
    # next sweep to retry (not cleared).
    assert n == 1
    assert record == ["cancelled"]
    assert len(infra.db.executed) == 1
    assert infra.db.executed[0][1] == ("j2", "t1")


@pytest.mark.asyncio
async def test_no_orphans_is_noop(monkeypatch):
    record = []
    _install_fake_modal(monkeypatch, record)
    infra = _infra([])

    n = await cancel_orphaned_gpu_calls(infra)

    assert n == 0
    assert infra.db.executed == []
