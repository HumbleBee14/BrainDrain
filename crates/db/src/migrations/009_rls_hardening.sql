-- Migration 009: RLS Policy Hardening
--
-- Problem: The existing RLS policies use:
--   USING (tenant_id = current_setting('app.tenant_id', true)::uuid)
--
-- When app.tenant_id is '' (empty string, set by pool before_acquire),
-- casting '' to uuid raises an error. When it is NULL (missing_ok=true returns
-- NULL), tenant_id = NULL is always false — correct but silent.
--
-- This migration replaces all policies with a version that:
--   1. Returns false (no rows) when app.tenant_id is empty or NULL (fail closed)
--   2. Casts to uuid only when the setting is a non-empty string
--   3. Works correctly whether the app uses SET LOCAL or SET SESSION
--
-- The helper function app_tenant_id() encapsulates this logic so policy
-- expressions remain concise. It is marked SECURITY DEFINER so it always
-- runs as the function owner, not the calling role.

CREATE OR REPLACE FUNCTION app_tenant_id()
RETURNS uuid
LANGUAGE plpgsql
STABLE
SECURITY DEFINER
AS $$
DECLARE
    raw text;
BEGIN
    raw := current_setting('app.tenant_id', true);
    IF raw IS NULL OR raw = '' THEN
        RETURN NULL;  -- NULL = no rows pass (fail closed)
    END IF;
    RETURN raw::uuid;
EXCEPTION WHEN invalid_text_representation THEN
    RETURN NULL;  -- Malformed UUID also fails closed
END;
$$;

-- Drop and recreate all RLS policies to use the safe helper function.
-- The existing policies were created in migrations 002, 004, 005, 007, 008.

-- ── projects ──
DROP POLICY IF EXISTS tenant_isolation_projects ON projects;
CREATE POLICY tenant_isolation_projects ON projects
    USING (tenant_id = app_tenant_id());

-- ── documents ──
DROP POLICY IF EXISTS tenant_isolation_documents ON documents;
CREATE POLICY tenant_isolation_documents ON documents
    USING (tenant_id = app_tenant_id());

-- ── datasets ──
DROP POLICY IF EXISTS tenant_isolation_datasets ON datasets;
CREATE POLICY tenant_isolation_datasets ON datasets
    USING (tenant_id = app_tenant_id());

-- ── training_jobs ──
DROP POLICY IF EXISTS tenant_isolation_training_jobs ON training_jobs;
CREATE POLICY tenant_isolation_training_jobs ON training_jobs
    USING (tenant_id = app_tenant_id());

-- ── models ──
DROP POLICY IF EXISTS tenant_isolation_models ON models;
CREATE POLICY tenant_isolation_models ON models
    USING (tenant_id = app_tenant_id());

-- ── evaluations ──
DROP POLICY IF EXISTS tenant_isolation_evaluations ON evaluations;
CREATE POLICY tenant_isolation_evaluations ON evaluations
    USING (tenant_id = app_tenant_id());

-- ── api_keys ──
DROP POLICY IF EXISTS tenant_isolation_api_keys ON api_keys;
CREATE POLICY tenant_isolation_api_keys ON api_keys
    USING (tenant_id = app_tenant_id());

-- ── billing_events ──
DROP POLICY IF EXISTS tenant_isolation_billing_events ON billing_events;
CREATE POLICY tenant_isolation_billing_events ON billing_events
    USING (tenant_id = app_tenant_id());

-- ── audit_logs ──
DROP POLICY IF EXISTS tenant_isolation_audit_logs ON audit_logs;
CREATE POLICY tenant_isolation_audit_logs ON audit_logs
    USING (tenant_id = app_tenant_id());

-- ── team_members ──
DROP POLICY IF EXISTS tenant_isolation_team_members ON team_members;
CREATE POLICY tenant_isolation_team_members ON team_members
    USING (tenant_id = app_tenant_id());

-- ── invitations ──
DROP POLICY IF EXISTS tenant_isolation_invitations ON invitations;
CREATE POLICY tenant_isolation_invitations ON invitations
    USING (tenant_id = app_tenant_id());

-- ── notification_preferences ──
DROP POLICY IF EXISTS tenant_isolation_notification_preferences ON notification_preferences;
CREATE POLICY tenant_isolation_notification_preferences ON notification_preferences
    USING (tenant_id = app_tenant_id());

-- ── notification_deliveries ──
DROP POLICY IF EXISTS tenant_isolation_notification_deliveries ON notification_deliveries;
CREATE POLICY tenant_isolation_notification_deliveries ON notification_deliveries
    USING (tenant_id = app_tenant_id());

-- ── model_exports ──
DROP POLICY IF EXISTS tenant_isolation_model_exports ON model_exports;
CREATE POLICY tenant_isolation_model_exports ON model_exports
    USING (tenant_id = app_tenant_id());
