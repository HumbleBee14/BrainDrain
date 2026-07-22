-- Data Studio: guided synthetic data generation
-- Tracks the guidance/facets/preview workflow that produces a dataset.

CREATE TABLE IF NOT EXISTS data_guides (
    id                    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id             UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    project_id            UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    task_type             VARCHAR(50) NOT NULL DEFAULT 'question_answering',
    status                VARCHAR(50) NOT NULL DEFAULT 'draft',
    guidance              TEXT NOT NULL DEFAULT '',
    facets                JSONB NOT NULL DEFAULT '[]',
    preview_samples       JSONB NOT NULL DEFAULT '[]',
    refinement_history    JSONB NOT NULL DEFAULT '[]',
    config                JSONB NOT NULL DEFAULT '{}',
    dataset_id            UUID REFERENCES datasets(id) ON DELETE SET NULL,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_data_guides_project ON data_guides(project_id, created_at DESC);

ALTER TABLE data_guides ENABLE ROW LEVEL SECURITY;

CREATE POLICY tenant_isolation_data_guides ON data_guides
    USING (tenant_id = app_tenant_id());

CREATE OR REPLACE TRIGGER set_updated_at_data_guides
BEFORE UPDATE ON data_guides
FOR EACH ROW
EXECUTE FUNCTION update_updated_at_column();
