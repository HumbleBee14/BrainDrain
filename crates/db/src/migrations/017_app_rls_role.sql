-- Migration 017: Least-privilege application role for RLS enforcement
--
-- Multi-tenant isolation is enforced by two things working together:
--   1. Every query carries `WHERE tenant_id = $N` (application-level).
--   2. Row-Level Security policies keyed on `app.tenant_id` (database-level),
--      which fail closed when no tenant context is set.
--
-- For (2) to actually take effect, the connection running tenant queries must
-- be a role that is SUBJECT to RLS. PostgreSQL exempts three kinds of roles:
-- superusers, roles with BYPASSRLS, and a table's OWNER (unless the table is
-- set to FORCE ROW LEVEL SECURITY). The migration runner / owner role is one of
-- those exempt roles, so running tenant traffic as the owner silently disables
-- RLS.
--
-- This migration introduces a dedicated `app_rls` role that is none of those:
-- NOSUPERUSER, NOBYPASSRLS, and not the table owner. The application connects
-- as `app_rls` for all tenant request traffic (DATABASE_RLS_URL) and keeps the
-- owner connection (DATABASE_URL) for migrations, partition DDL, and the small
-- set of legitimately cross-tenant operations.
--
-- We deliberately do NOT use FORCE ROW LEVEL SECURITY: the owner connection must
-- remain able to bypass RLS for cross-tenant maintenance (billing batch writes
-- that span tenants, the stale-deployment reaper, the global adapter-cap count)
-- and to run partition DDL that only the owner may perform.
--
-- PRODUCTION: provision the `app_rls` role with a strong password BEFORE running
-- migrations. The DO block below only creates the role if it is absent, so a
-- pre-provisioned role keeps its own password. The literal password here is a
-- LOCAL DEVELOPMENT default only.

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'app_rls') THEN
        CREATE ROLE app_rls
            LOGIN
            PASSWORD 'app_rls_dev_password'  -- dev only; override in production
            NOSUPERUSER
            NOBYPASSRLS
            NOCREATEDB
            NOCREATEROLE;
    END IF;
END
$$;

-- Let app_rls reach objects in the public schema.
GRANT USAGE ON SCHEMA public TO app_rls;

-- Data-plane privileges. RLS policies still constrain which rows app_rls can
-- see/modify on the 15 tenant tables; tables without RLS (tenants,
-- inference_instances, idempotency_keys, billing_outbox) are reachable as today.
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO app_rls;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO app_rls;
GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA public TO app_rls;

-- Future tables/sequences/functions created by the owner should also be usable
-- by app_rls without another grant migration.
ALTER DEFAULT PRIVILEGES IN SCHEMA public
    GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO app_rls;
ALTER DEFAULT PRIVILEGES IN SCHEMA public
    GRANT USAGE, SELECT ON SEQUENCES TO app_rls;
ALTER DEFAULT PRIVILEGES IN SCHEMA public
    GRANT EXECUTE ON FUNCTIONS TO app_rls;
