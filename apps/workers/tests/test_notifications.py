"""Tests for the worker notification enqueue helper (no real DB)."""

import json

import pytest

from src.notifications import (
    EVENT_EVALUATION_COMPLETE,
    EVENT_TRAINING_COMPLETE,
    enqueue_notification,
)


class FakeConn:
    """Minimal asyncpg-connection stand-in recording fetch/execute calls."""

    def __init__(self, prefs):
        self._prefs = prefs
        self.fetch_calls = []
        self.inserts = []

    async def fetch(self, query, *args):
        self.fetch_calls.append((query, args))
        return self._prefs

    async def execute(self, query, *args):
        self.inserts.append((query, args))


@pytest.mark.asyncio
async def test_enqueues_one_delivery_per_enabled_preference():
    tenant = "11111111-1111-1111-1111-111111111111"
    prefs = [
        {"id": "aaaa", "channel": "email"},
        {"id": "bbbb", "channel": "webhook"},
    ]
    conn = FakeConn(prefs)

    count = await enqueue_notification(
        conn,
        tenant_id=tenant,
        event_type=EVENT_TRAINING_COMPLETE,
        payload={"status": "completed", "subject": "s", "message": "m"},
    )

    assert count == 2
    assert len(conn.inserts) == 2
    # Preferences are filtered by the enabled event type on the tenant.
    assert conn.fetch_calls[0][1] == (tenant, EVENT_TRAINING_COMPLETE)
    # Each insert binds tenant, preference id, event type, channel, json payload.
    for insert, pref in zip(conn.inserts, prefs):
        _, args = insert
        assert args[0] == tenant
        assert args[1] == pref["id"]
        assert args[2] == EVENT_TRAINING_COMPLETE
        assert args[3] == pref["channel"]
        assert json.loads(args[4])["status"] == "completed"


@pytest.mark.asyncio
async def test_no_preferences_enqueues_nothing():
    conn = FakeConn([])
    count = await enqueue_notification(
        conn,
        tenant_id="t",
        event_type=EVENT_EVALUATION_COMPLETE,
        payload={"status": "failed"},
    )
    assert count == 0
    assert conn.inserts == []
