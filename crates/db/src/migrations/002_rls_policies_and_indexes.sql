-- Migration 002: RLS Policies and Performance Indexes
--
-- Adds:
--   1. RLS policies for multi-tenant isolation (all tables)
--   2. Composite indexes for common query patterns
--
-- RLS was enabled in migration 001 but had zero policies.
-- These policies enforce that a session variable `app.tenant_id`
-- (set by the application per-request) restricts all access to
-- matching tenant_id rows.
--
-- For the application user (non-superuser), every query automatically
-- filters by tenant_id. Superusers bypass RLS by default.

-- ========================================
-- RLS POLICIES
-- ========================================

-- Projects: tenant can only see their own projects
CREATE POLICY tenant_isolation_projects ON projects
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);

-- Documents: tenant can only see their own documents
CREATE POLICY tenant_isolation_documents ON documents
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);

-- Datasets: tenant can only see their own datasets
CREATE POLICY tenant_isolation_datasets ON datasets
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);

-- Training jobs: tenant can only see their own training jobs
CREATE POLICY tenant_isolation_training_jobs ON training_jobs
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);

-- Models: tenant can only see their own models
CREATE POLICY tenant_isolation_models ON models
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);

-- Evaluations: tenant can only see their own evaluations
CREATE POLICY tenant_isolation_evaluations ON evaluations
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);

-- API keys: tenant can only see their own API keys
CREATE POLICY tenant_isolation_api_keys ON api_keys
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);

-- Billing events: tenant can only see their own billing events
CREATE POLICY tenant_isolation_billing_events ON billing_events
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);

-- ========================================
-- PERFORMANCE INDEXES
-- ========================================

-- Documents: filter by project + status (used by pipeline triggers)
CREATE INDEX IF NOT EXISTS idx_documents_project_status
    ON documents(project_id, status);

-- Documents: filter by tenant + project (used by list queries)
CREATE INDEX IF NOT EXISTS idx_documents_tenant_project
    ON documents(tenant_id, project_id, created_at DESC);

-- Datasets: filter by project + status (used by pipeline status)
CREATE INDEX IF NOT EXISTS idx_datasets_project_status
    ON datasets(project_id, status);

-- Training jobs: filter by project + status (used by pipeline status)
CREATE INDEX IF NOT EXISTS idx_training_jobs_project_status
    ON training_jobs(project_id, status);

-- Training jobs: filter by tenant + project (used by list queries)
CREATE INDEX IF NOT EXISTS idx_training_jobs_tenant_project
    ON training_jobs(tenant_id, project_id, created_at DESC);

-- Models: filter by tenant + project (used by list queries)
CREATE INDEX IF NOT EXISTS idx_models_tenant_project
    ON models(tenant_id, project_id, created_at DESC);

-- Models: filter by deployment status (used by deployment queries)
CREATE INDEX IF NOT EXISTS idx_models_deployment_status
    ON models(tenant_id, project_id, deployment_status);

-- Evaluations: filter by model + creation time (used by list queries)
CREATE INDEX IF NOT EXISTS idx_evaluations_model_created
    ON evaluations(model_id, created_at DESC);

-- Evaluations: filter by project + status (used by pipeline status)
-- Requires join through models, but this index helps direct lookups
CREATE INDEX IF NOT EXISTS idx_evaluations_tenant_status
    ON evaluations(tenant_id, status);

-- API keys: lookup by key hash (used by authentication)
CREATE INDEX IF NOT EXISTS idx_api_keys_hash
    ON api_keys(key_hash) WHERE is_active = true;

-- API keys: list by model (used by management UI)
CREATE INDEX IF NOT EXISTS idx_api_keys_model
    ON api_keys(tenant_id, model_id);
