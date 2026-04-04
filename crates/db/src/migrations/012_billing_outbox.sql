-- Durable billing outbox: transactional write surface for billing events.
--
-- All billing-producing operations (inference, deploy, training) write here
-- first. A background relay worker moves rows into billing_events (the
-- reporting ledger) using SELECT FOR UPDATE SKIP LOCKED.
--
-- This ensures no billing event is lost on API crash — the row is committed
-- before the handler returns.

CREATE TABLE IF NOT EXISTS billing_outbox (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID NOT NULL,
    operation       TEXT NOT NULL,
    resource_id     UUID,
    tokens_in       BIGINT NOT NULL DEFAULT 0,
    tokens_out      BIGINT NOT NULL DEFAULT 0,
    gpu_seconds     INT NOT NULL DEFAULT 0,
    cost_usd        DOUBLE PRECISION NOT NULL DEFAULT 0,
    metadata        JSONB NOT NULL DEFAULT '{}',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    delivered_at    TIMESTAMPTZ,
    attempt_count   INT NOT NULL DEFAULT 0,
    last_error      TEXT,

    CONSTRAINT chk_billing_outbox_cost CHECK (cost_usd >= 0)
);

-- Relay claims undelivered rows ordered by creation time.
CREATE INDEX idx_billing_outbox_pending
    ON billing_outbox (created_at)
    WHERE delivered_at IS NULL;

-- Cleanup of delivered rows (older than 7 days).
CREATE INDEX idx_billing_outbox_delivered
    ON billing_outbox (delivered_at)
    WHERE delivered_at IS NOT NULL;
