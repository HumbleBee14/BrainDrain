-- Multi-instance inference control plane
-- Tracks serving instances explicitly and binds deployed models to an instance.

CREATE TABLE IF NOT EXISTS inference_instances (
    id                    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name                  VARCHAR(255) NOT NULL,
    base_url              VARCHAR(1000) NOT NULL UNIQUE,
    backend_type          VARCHAR(50) NOT NULL,
    gpu_class             VARCHAR(50),
    base_model            VARCHAR(255) NOT NULL,
    max_adapters          INTEGER NOT NULL DEFAULT 4 CHECK (max_adapters > 0),
    active_adapter_count  INTEGER NOT NULL DEFAULT 0 CHECK (active_adapter_count >= 0),
    health_status         VARCHAR(50) NOT NULL DEFAULT 'unknown',
    lifecycle_state       VARCHAR(50) NOT NULL DEFAULT 'ready',
    last_health_check_at  TIMESTAMPTZ,
    last_healthy_at       TIMESTAMPTZ,
    metadata              JSONB NOT NULL DEFAULT '{}',
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_inference_instances_scheduling
    ON inference_instances (backend_type, base_model, active_adapter_count, created_at)
    WHERE lifecycle_state = 'ready' AND health_status = 'healthy';

CREATE INDEX IF NOT EXISTS idx_inference_instances_status
    ON inference_instances (lifecycle_state, health_status, updated_at DESC);

ALTER TABLE models
    ADD COLUMN IF NOT EXISTS inference_instance_id UUID REFERENCES inference_instances(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_models_inference_instance
    ON models (inference_instance_id)
    WHERE inference_instance_id IS NOT NULL;

CREATE OR REPLACE TRIGGER set_updated_at_inference_instances
BEFORE UPDATE ON inference_instances
FOR EACH ROW
EXECUTE FUNCTION update_updated_at_column();
