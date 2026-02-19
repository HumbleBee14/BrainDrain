-- Initial Schema
-- All tables use UUID primary keys, TIMESTAMPTZ, and tenant_id for multi-tenancy.

-- ========================================
-- TENANTS (maps to Clerk organizations)
-- ========================================
CREATE TABLE IF NOT EXISTS tenants (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    clerk_org_id    VARCHAR(255) UNIQUE NOT NULL,
    name            VARCHAR(255) NOT NULL,
    plan            VARCHAR(50) NOT NULL DEFAULT 'starter',
    settings        JSONB NOT NULL DEFAULT '{}',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ========================================
-- PROJECTS
-- ========================================
CREATE TABLE IF NOT EXISTS projects (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    name            VARCHAR(255) NOT NULL,
    description     TEXT,
    task_type       VARCHAR(50),
    config          JSONB NOT NULL DEFAULT '{}',
    status          VARCHAR(50) NOT NULL DEFAULT 'created',
    deleted_at      TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_projects_tenant ON projects(tenant_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_projects_status ON projects(tenant_id, status);

-- ========================================
-- DOCUMENTS
-- ========================================
CREATE TABLE IF NOT EXISTS documents (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    project_id      UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    filename        VARCHAR(500) NOT NULL,
    file_size       BIGINT NOT NULL,
    mime_type       VARCHAR(100) NOT NULL,
    storage_path    VARCHAR(1000) NOT NULL,
    status          VARCHAR(50) NOT NULL DEFAULT 'uploaded',
    parse_quality   DOUBLE PRECISION,
    page_count      INTEGER,
    language        VARCHAR(10),
    domain          VARCHAR(50),
    metadata        JSONB NOT NULL DEFAULT '{}',
    error_message   TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_documents_project ON documents(project_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_documents_status ON documents(tenant_id, status);

-- ========================================
-- DATASETS
-- ========================================
CREATE TABLE IF NOT EXISTS datasets (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    project_id      UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name            VARCHAR(255) NOT NULL,
    storage_path    VARCHAR(1000),
    format          VARCHAR(50) NOT NULL DEFAULT 'chatml',
    status          VARCHAR(50) NOT NULL DEFAULT 'generating',
    pair_count      INTEGER DEFAULT 0,
    stats           JSONB NOT NULL DEFAULT '{}',
    config          JSONB NOT NULL DEFAULT '{}',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_datasets_project ON datasets(project_id, created_at DESC);

-- ========================================
-- TRAINING JOBS
-- ========================================
CREATE TABLE IF NOT EXISTS training_jobs (
    id                      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id               UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    project_id              UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    dataset_id              UUID NOT NULL REFERENCES datasets(id),
    base_model              VARCHAR(255) NOT NULL,
    method                  VARCHAR(50) NOT NULL DEFAULT 'qlora',
    mode                    VARCHAR(50) NOT NULL DEFAULT 'quick',
    hyperparams             JSONB NOT NULL DEFAULT '{}',
    gpu_class               VARCHAR(50),
    status                  VARCHAR(50) NOT NULL DEFAULT 'pending',
    cost_estimate           DECIMAL(10,2),
    actual_cost             DECIMAL(10,2),
    metrics                 JSONB NOT NULL DEFAULT '{}',
    started_at              TIMESTAMPTZ,
    completed_at            TIMESTAMPTZ,
    temporal_workflow_id    VARCHAR(255),
    error_message           TEXT,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_training_jobs_project ON training_jobs(project_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_training_jobs_status ON training_jobs(tenant_id, status);

-- ========================================
-- MODELS (trained adapters)
-- ========================================
CREATE TABLE IF NOT EXISTS models (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    project_id          UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    training_job_id     UUID NOT NULL REFERENCES training_jobs(id),
    name                VARCHAR(255) NOT NULL,
    base_model          VARCHAR(255) NOT NULL,
    adapter_path        VARCHAR(1000),
    adapter_size_bytes  BIGINT,
    deployment_status   VARCHAR(50) NOT NULL DEFAULT 'undeployed',
    deployment_config   JSONB NOT NULL DEFAULT '{}',
    eval_scores         JSONB NOT NULL DEFAULT '{}',
    version             INTEGER NOT NULL DEFAULT 1,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_models_project ON models(project_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_models_deployment ON models(tenant_id, deployment_status);

-- ========================================
-- EVALUATIONS
-- ========================================
CREATE TABLE IF NOT EXISTS evaluations (
    id                      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id               UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    model_id                UUID NOT NULL REFERENCES models(id) ON DELETE CASCADE,
    status                  VARCHAR(50) NOT NULL DEFAULT 'running',
    scores                  JSONB NOT NULL DEFAULT '{}',
    report                  JSONB NOT NULL DEFAULT '{}',
    temporal_workflow_id    VARCHAR(255),
    started_at              TIMESTAMPTZ,
    completed_at            TIMESTAMPTZ,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_evaluations_model ON evaluations(model_id, created_at DESC);

-- ========================================
-- API KEYS (for model inference)
-- ========================================
CREATE TABLE IF NOT EXISTS api_keys (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    model_id        UUID NOT NULL REFERENCES models(id) ON DELETE CASCADE,
    name            VARCHAR(255) NOT NULL,
    key_prefix      VARCHAR(10) NOT NULL,
    key_hash        VARCHAR(255) NOT NULL UNIQUE,
    rate_limit      INTEGER NOT NULL DEFAULT 60,
    is_active       BOOLEAN NOT NULL DEFAULT TRUE,
    last_used_at    TIMESTAMPTZ,
    expires_at      TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_api_keys_hash ON api_keys(key_hash) WHERE is_active = TRUE;
CREATE INDEX IF NOT EXISTS idx_api_keys_model ON api_keys(model_id);

-- ========================================
-- BILLING / USAGE LEDGER (append-only)
-- ========================================
CREATE TABLE IF NOT EXISTS billing_events (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    operation       VARCHAR(50) NOT NULL,
    resource_id     UUID,
    tokens_in       BIGINT DEFAULT 0,
    tokens_out      BIGINT DEFAULT 0,
    gpu_seconds     INTEGER DEFAULT 0,
    cost_usd        DECIMAL(10,4) NOT NULL DEFAULT 0,
    metadata        JSONB NOT NULL DEFAULT '{}',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_billing_tenant ON billing_events(tenant_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_billing_operation ON billing_events(tenant_id, operation, created_at DESC);

-- ========================================
-- ROW-LEVEL SECURITY
-- ========================================
ALTER TABLE projects ENABLE ROW LEVEL SECURITY;
ALTER TABLE documents ENABLE ROW LEVEL SECURITY;
ALTER TABLE datasets ENABLE ROW LEVEL SECURITY;
ALTER TABLE training_jobs ENABLE ROW LEVEL SECURITY;
ALTER TABLE models ENABLE ROW LEVEL SECURITY;
ALTER TABLE evaluations ENABLE ROW LEVEL SECURITY;
ALTER TABLE api_keys ENABLE ROW LEVEL SECURITY;
ALTER TABLE billing_events ENABLE ROW LEVEL SECURITY;

-- ========================================
-- UPDATED_AT TRIGGER FUNCTION
-- ========================================
CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Apply updated_at trigger to all tables with updated_at
DO $$
DECLARE
    t TEXT;
BEGIN
    FOR t IN SELECT unnest(ARRAY[
        'tenants', 'projects', 'documents', 'datasets',
        'training_jobs', 'models', 'evaluations'
    ]) LOOP
        EXECUTE format(
            'CREATE OR REPLACE TRIGGER set_updated_at BEFORE UPDATE ON %I FOR EACH ROW EXECUTE FUNCTION update_updated_at_column()',
            t
        );
    END LOOP;
END;
$$;
