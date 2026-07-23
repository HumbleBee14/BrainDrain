-- Migration 020: retry backoff + lease for notification delivery.
--
-- `next_retry_at` serves two purposes on the delivery worker's claim query:
--   1. Backoff: a failed delivery sets it to NOW() + an exponential delay, so the
--      worker does not hot-loop retries on the next poll.
--   2. Lease: at claim time the worker pushes it into the near future so a
--      concurrent worker (SELECT ... FOR UPDATE SKIP LOCKED) does not pick the
--      same row. If the worker crashes mid-dispatch the lease expires and the row
--      becomes eligible again — no separate in-flight status to get stuck.
ALTER TABLE notification_deliveries ADD COLUMN IF NOT EXISTS next_retry_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_notification_deliveries_retry
    ON notification_deliveries (status, next_retry_at);
