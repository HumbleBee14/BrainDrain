-- Data flywheel: capture production inference traffic for review + feedback.
-- Capture is opt-in per model (models.capture_traffic, default off).

ALTER TABLE models ADD COLUMN IF NOT EXISTS capture_traffic BOOLEAN NOT NULL DEFAULT FALSE;

CREATE TABLE IF NOT EXISTS inference_samples (
    id              UUID PRIMARY KEY,
    tenant_id       UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    model_id        UUID NOT NULL REFERENCES models(id) ON DELETE CASCADE,
    api_key_id      UUID,
    messages        JSONB NOT NULL,
    response        TEXT NOT NULL,
    rating          VARCHAR(20),
    rating_comment  TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_inference_samples_model
    ON inference_samples(model_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_inference_samples_rated
    ON inference_samples(model_id, rating) WHERE rating IS NOT NULL;

ALTER TABLE inference_samples ENABLE ROW LEVEL SECURITY;

CREATE POLICY tenant_isolation_inference_samples ON inference_samples
    USING (tenant_id = app_tenant_id());

CREATE OR REPLACE TRIGGER set_updated_at_inference_samples
BEFORE UPDATE ON inference_samples
FOR EACH ROW
EXECUTE FUNCTION update_updated_at_column();
