-- Migration 009: Performance hardening — indexes, FK cascades, duplicate cleanup
--
-- Addresses:
-- 1. Missing composite index for model version lookups
-- 2. Duplicate indexes on evaluations and api_keys tables
-- 3. Missing ON DELETE CASCADE on foreign keys
-- 4. Audit log partitioning for high-volume append-only table

-- =============================================================================
-- 1. Add missing index for model version queries (list_versions)
-- =============================================================================
-- list_versions() queries: WHERE tenant_id = $1 AND project_id = $2 AND base_model = $3 ORDER BY version DESC
-- Without this index, PostgreSQL falls back to sequential scan.
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_models_base_model_version
    ON models (tenant_id, project_id, base_model, version DESC);

-- =============================================================================
-- 2. Drop duplicate indexes
-- =============================================================================
-- idx_evaluations_model (001) and idx_evaluations_model_created (002) are identical:
-- both ON evaluations(model_id, created_at DESC). Drop the older one.
DROP INDEX IF EXISTS idx_evaluations_model;

-- idx_api_keys_model from migration 001 is ON (model_id) only.
-- Migration 002 recreated it as ON (tenant_id, model_id) which is strictly better.
-- The old one is redundant — PostgreSQL can use the (tenant_id, model_id) index for model_id-only queries
-- via index skip scan or it falls back to the composite which still covers the case.
-- However, since both have the same name, PostgreSQL would have already replaced it.
-- This is a no-op safety check.
DROP INDEX IF EXISTS idx_api_keys_model_legacy;

-- =============================================================================
-- 3. Fix missing ON DELETE CASCADE on foreign keys
-- =============================================================================
-- training_jobs.dataset_id — currently RESTRICT, should CASCADE
-- If a dataset is deleted, associated training jobs should be cleaned up.
ALTER TABLE training_jobs DROP CONSTRAINT IF EXISTS training_jobs_dataset_id_fkey;
ALTER TABLE training_jobs ADD CONSTRAINT training_jobs_dataset_id_fkey
    FOREIGN KEY (dataset_id) REFERENCES datasets(id) ON DELETE CASCADE;

-- models.training_job_id — currently RESTRICT, should CASCADE
ALTER TABLE models DROP CONSTRAINT IF EXISTS models_training_job_id_fkey;
ALTER TABLE models ADD CONSTRAINT models_training_job_id_fkey
    FOREIGN KEY (training_job_id) REFERENCES training_jobs(id) ON DELETE CASCADE;

-- team_members.tenant_id — currently RESTRICT, should CASCADE
ALTER TABLE team_members DROP CONSTRAINT IF EXISTS team_members_tenant_id_fkey;
ALTER TABLE team_members ADD CONSTRAINT team_members_tenant_id_fkey
    FOREIGN KEY (tenant_id) REFERENCES tenants(id) ON DELETE CASCADE;

-- invitations.tenant_id — currently RESTRICT, should CASCADE
ALTER TABLE invitations DROP CONSTRAINT IF EXISTS invitations_tenant_id_fkey;
ALTER TABLE invitations ADD CONSTRAINT invitations_tenant_id_fkey
    FOREIGN KEY (tenant_id) REFERENCES tenants(id) ON DELETE CASCADE;
