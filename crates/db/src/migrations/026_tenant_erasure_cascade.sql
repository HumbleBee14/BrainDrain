-- Tenant erasure (GDPR right-to-erasure / offboarding).
--
-- These three tables FK tenants(id) WITHOUT an ON DELETE action, so they
-- RESTRICT: a `DELETE FROM tenants` would fail while any row remains. They
-- hold operational data that MUST be wiped when a tenant is erased, so switch
-- them to ON DELETE CASCADE. A tenant delete then cascades all operational
-- tables (matching projects, documents, models, ... which already cascade).
--
-- Intentionally NOT touched: billing_events (partitioned ledger), billing_outbox,
-- and audit_logs carry no tenant FK, so a tenant delete leaves them in place.
-- That retention is deliberate — financial and audit records survive erasure.

ALTER TABLE model_exports DROP CONSTRAINT IF EXISTS model_exports_tenant_id_fkey;
ALTER TABLE model_exports ADD CONSTRAINT model_exports_tenant_id_fkey
    FOREIGN KEY (tenant_id) REFERENCES tenants(id) ON DELETE CASCADE;

ALTER TABLE notification_preferences DROP CONSTRAINT IF EXISTS notification_preferences_tenant_id_fkey;
ALTER TABLE notification_preferences ADD CONSTRAINT notification_preferences_tenant_id_fkey
    FOREIGN KEY (tenant_id) REFERENCES tenants(id) ON DELETE CASCADE;

ALTER TABLE notification_deliveries DROP CONSTRAINT IF EXISTS notification_deliveries_tenant_id_fkey;
ALTER TABLE notification_deliveries ADD CONSTRAINT notification_deliveries_tenant_id_fkey
    FOREIGN KEY (tenant_id) REFERENCES tenants(id) ON DELETE CASCADE;
