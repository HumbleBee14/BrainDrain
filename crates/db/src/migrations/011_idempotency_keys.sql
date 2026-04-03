-- Idempotency key storage for safe client retries on mutating endpoints.
--
-- Scoped per principal (JWT sub claim) + method + route to prevent cross-user
-- and cross-endpoint replay. Keys expire after 24 hours.
-- Stale processing keys reaped after 5 minutes (crash recovery).

CREATE TABLE IF NOT EXISTS idempotency_keys (
    id                     UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    principal_id           TEXT NOT NULL,
    idempotency_key        TEXT NOT NULL,
    method                 TEXT NOT NULL,
    route                  TEXT NOT NULL,
    request_hash           TEXT NOT NULL,
    status                 TEXT NOT NULL DEFAULT 'processing',  -- processing | completed | failed
    response_status        SMALLINT,
    response_content_type  TEXT,
    response_body          BYTEA,
    created_at             TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at           TIMESTAMPTZ,
    expires_at             TIMESTAMPTZ NOT NULL DEFAULT NOW() + INTERVAL '24 hours',

    -- Unique per principal + key + method + route: prevents cross-endpoint key reuse
    CONSTRAINT uq_idempotency_principal_key
        UNIQUE (principal_id, idempotency_key, method, route)
);

-- Cleanup: all expired keys (any status, including stuck processing).
CREATE INDEX idx_idempotency_keys_expires_at ON idempotency_keys (expires_at);

-- Fast lookup for stale processing keys (crash recovery).
CREATE INDEX idx_idempotency_keys_stale_processing ON idempotency_keys (created_at)
    WHERE status = 'processing';
