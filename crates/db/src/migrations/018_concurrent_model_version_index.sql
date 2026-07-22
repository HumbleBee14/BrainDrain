-- no-transaction
-- Migration 018: Non-blocking creation of the model version-lookup index.
--
-- Serves list_versions(): WHERE tenant_id = $1 AND project_id = $2
-- AND base_model = $3 ORDER BY version DESC.
--
-- Split out of migration 009 because CREATE INDEX CONCURRENTLY cannot run inside
-- a transaction, and sqlx wraps each migration in one by default. The leading
-- `-- no-transaction` line (honored by sqlx) makes this migration run outside a
-- transaction so CONCURRENTLY is allowed. Idempotent via IF NOT EXISTS.
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_models_base_model_version
    ON models (tenant_id, project_id, base_model, version DESC);
