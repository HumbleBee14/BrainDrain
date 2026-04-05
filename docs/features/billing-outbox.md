# Durable Billing Outbox

**PR:** #22  
**Problem:** If the server crashes right after processing an inference request
but before recording the billing event, that usage is lost forever — the
customer used the service but was never charged.

## How it works

Instead of sending billing events directly to the reporting table (which
could fail independently), we write them to an **outbox table** in the same
database transaction as the business operation. A background relay worker
then picks up outbox rows and delivers them to the reporting ledger.

```
Request → Business logic + billing row (one transaction) → Response
                                          ↓
                               Background relay picks up row
                                          ↓
                               Delivers to billing_events table
```

**The guarantee:** If the business operation committed (user got a 200), the
billing row is on disk. Even if the server crashes immediately after, the
relay will deliver it when it comes back up.

## Key concepts

- **Outbox table (`billing_outbox`):** Where billing events are first written.
  Same transaction as the business logic. This is the durability guarantee.

- **Relay worker:** Background task that moves rows from outbox → ledger.
  Uses `FOR UPDATE SKIP LOCKED` so multiple instances don't fight over rows.

- **Idempotent delivery:** `ON CONFLICT DO NOTHING` prevents double-charging
  if the relay delivers a row twice (e.g., crash between deliver and mark).

- **Per-row savepoints:** One bad row doesn't block the entire batch.

- **Streaming reservation:** For streaming inference, a "pending" row with
  a conservative fallback charge is written BEFORE the stream starts.
  After the stream completes, it's updated with actual token counts.

## Where billing events are produced

| Producer | Durable? | How |
|---|---|---|
| Inference (single/batch) | Yes | `record_billing_event_required` in handler |
| Streaming inference | Yes | Pending row before stream, finalized after |
| Model deployment | Yes | Same transaction as deployment state change |
| Training (success/failure) | Yes | Python worker writes outbox row in same DB transaction |

## When to use `_required` vs `_best_effort`

- `record_billing_event_required` — Returns `Result`. Use for anything
  financial. If the outbox write fails, the handler returns an error.
- `record_billing_event_best_effort` — Logs and swallows. Use for
  non-critical metrics where losing one event isn't a business problem.

## Files

- `crates/api/src/services/billing_outbox.rs` — Outbox implementation + relay
- `crates/api/src/app_state.rs` — `record_billing_event_*` methods
- `crates/db/src/migrations/012_billing_outbox.sql` — Schema
- `docs/BILLING_ARCHITECTURE.md` — Full technical reference
