-- Idempotency key storage for safe client retries on mutating endpoints.
--
-- Scoped per principal (JWT sub claim) to prevent cross-user replay.
-- Keys expire after 24 hours (cleaned up by background task).
-- In-flight requests tracked via status to prevent concurrent duplicates.

CREATE TABLE IF NOT EXISTS idempotency_keys (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    principal_id    TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    method          TEXT NOT NULL,
    route           TEXT NOT NULL,
    request_hash    TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'processing',  -- processing | completed | failed
    response_status SMALLINT,
    response_body   BYTEA,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at    TIMESTAMPTZ,
    expires_at      TIMESTAMPTZ NOT NULL DEFAULT NOW() + INTERVAL '24 hours',

    CONSTRAINT uq_idempotency_principal_key
        UNIQUE (principal_id, idempotency_key)
);

-- Cleanup job uses expires_at.
CREATE INDEX idx_idempotency_keys_expires_at ON idempotency_keys (expires_at)
    WHERE status != 'processing';
