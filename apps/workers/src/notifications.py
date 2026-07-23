"""Durably fan an event out to a tenant's enabled notification channels.

Mirrors the API's NotificationService.notify: for a given (tenant_id,
event_type), select the tenant's enabled preferences and insert one pending
`notification_deliveries` row per preference. The API's delivery worker then
dispatches them (email/webhook); in-app rows are read directly by the client.

The worker owns the ML terminal states (training/evaluation), so it is the
producer for those events. `enqueue_notification` takes an open connection and
must be called inside the caller's transaction, so the delivery rows commit
atomically with the state change that triggered them — no crash window between
"job completed" and "user notified".

Runs on the worker's owner DB connection, which bypasses RLS (same as the
billing outbox), so no app.tenant_id is required.
"""

import json

# Event-type keys — must match the defaults seeded by the API (auth.rs) and the
# preference keys used by the frontend.
EVENT_TRAINING_COMPLETE = "training_complete"
EVENT_EVALUATION_COMPLETE = "evaluation_complete"


async def enqueue_notification(
    conn,
    *,
    tenant_id: str,
    event_type: str,
    payload: dict,
) -> int:
    """Enqueue a pending delivery per enabled preference. Returns the count.

    Must run inside the caller's transaction so deliveries commit together with
    the announced state change.
    """
    prefs = await conn.fetch(
        """SELECT id, channel FROM notification_preferences
        WHERE tenant_id = $1 AND event_type = $2 AND enabled = true""",
        tenant_id,
        event_type,
    )
    payload_json = json.dumps(payload)
    for pref in prefs:
        await conn.execute(
            """INSERT INTO notification_deliveries
                (tenant_id, preference_id, event_type, channel, payload)
            VALUES ($1, $2, $3, $4, $5::jsonb)""",
            tenant_id,
            pref["id"],
            event_type,
            pref["channel"],
            payload_json,
        )
    return len(prefs)
