-- Migration 003: Billing Events Table Partitioning
--
-- Converts billing_events from a regular table to a monthly RANGE-partitioned
-- table on created_at. This is critical for query performance and data lifecycle
-- management as the append-only billing ledger grows.
--
-- Approach:
--   1. Rename existing table to billing_events_legacy
--   2. Drop the RLS policy on the legacy table (will be re-created on new table)
--   3. Create new partitioned table (no FK to tenants -- partitioned tables in
--      PostgreSQL do not fully support foreign keys in all versions; tenant_id
--      is enforced at the application layer)
--   4. Migrate existing data from legacy table
--   5. Drop legacy table
--   6. Re-create indexes, RLS, and RLS policy
--   7. Create initial monthly partitions
--   8. Create a helper function for future partition creation
--
-- NOTE: In production, use pg_cron or pg_partman to automatically create future
-- partitions ahead of time. Without automatic partition creation, inserts into
-- months without a partition will fail. A recommended pattern is a weekly cron
-- job that ensures partitions exist for the next 3 months.

BEGIN;

-- ========================================
-- STEP 1: Rename existing table to legacy
-- ========================================
ALTER TABLE billing_events RENAME TO billing_events_legacy;

-- Rename the indexes on the legacy table so they don't collide
ALTER INDEX IF EXISTS idx_billing_tenant RENAME TO idx_billing_tenant_legacy;
ALTER INDEX IF EXISTS idx_billing_operation RENAME TO idx_billing_operation_legacy;

-- ========================================
-- STEP 2: Drop RLS policy on legacy table
-- ========================================
-- The policy references the old table name; we re-create it on the new table.
DROP POLICY IF EXISTS tenant_isolation_billing_events ON billing_events_legacy;

-- ========================================
-- STEP 3: Create new partitioned table
-- ========================================
-- Key differences from the original:
--   - PRIMARY KEY includes created_at (required for partitioned tables)
--   - No REFERENCES tenants(id) FK (unsupported on partitioned tables;
--     tenant_id enforcement is the application layer's responsibility)
--   - PARTITION BY RANGE (created_at)
CREATE TABLE billing_events (
    id              UUID NOT NULL DEFAULT gen_random_uuid(),
    tenant_id       UUID NOT NULL,
    operation       VARCHAR(50) NOT NULL,
    resource_id     UUID,
    tokens_in       BIGINT DEFAULT 0,
    tokens_out      BIGINT DEFAULT 0,
    gpu_seconds     INTEGER DEFAULT 0,
    cost_usd        DECIMAL(10,4) NOT NULL DEFAULT 0,
    metadata        JSONB NOT NULL DEFAULT '{}',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (id, created_at)
) PARTITION BY RANGE (created_at);

-- ========================================
-- STEP 4: Re-create indexes on partitioned table
-- ========================================
-- These indexes are automatically created on each partition.
CREATE INDEX idx_billing_tenant
    ON billing_events (tenant_id, created_at DESC);

CREATE INDEX idx_billing_operation
    ON billing_events (tenant_id, operation, created_at DESC);

-- ========================================
-- STEP 5: Re-enable RLS and re-create policy
-- ========================================
ALTER TABLE billing_events ENABLE ROW LEVEL SECURITY;

CREATE POLICY tenant_isolation_billing_events ON billing_events
    USING (tenant_id = current_setting('app.tenant_id', true)::uuid);

-- ========================================
-- STEP 6: Create helper function for partition creation
-- ========================================
-- Creates a monthly partition for the given month. The partition name follows
-- the pattern: billing_events_YYYYMM (e.g., billing_events_202602).
--
-- Usage:   SELECT create_billing_partition('2026-02-01'::date);
-- Idempotent: silently does nothing if the partition already exists.
CREATE OR REPLACE FUNCTION create_billing_partition(month DATE)
RETURNS void
LANGUAGE plpgsql
AS $$
DECLARE
    partition_name TEXT;
    start_date DATE;
    end_date DATE;
BEGIN
    -- Normalize to first of month
    start_date := date_trunc('month', month)::date;
    end_date := (start_date + interval '1 month')::date;
    partition_name := 'billing_events_' || to_char(start_date, 'YYYYMM');

    -- Skip if partition already exists
    IF EXISTS (
        SELECT 1 FROM pg_class
        WHERE relname = partition_name
          AND relkind = 'r'
    ) THEN
        RETURN;
    END IF;

    EXECUTE format(
        'CREATE TABLE %I PARTITION OF billing_events FOR VALUES FROM (%L) TO (%L)',
        partition_name,
        start_date,
        end_date
    );
END;
$$;

-- ========================================
-- STEP 7: Create initial partitions
-- ========================================
-- Create partitions covering:
--   - All months that may contain legacy data (from 2024-01 to current month)
--   - Current month + next 3 months for headroom
--
-- We use a DO block with generate_series to cover a wide enough range that any
-- existing data is captured, plus future months.
DO $$
DECLARE
    m DATE;
    earliest DATE;
BEGIN
    -- Find the earliest created_at in legacy data (if any)
    SELECT date_trunc('month', MIN(created_at))::date
      INTO earliest
      FROM billing_events_legacy;

    -- If no legacy data, start from current month
    IF earliest IS NULL THEN
        earliest := date_trunc('month', now())::date;
    END IF;

    -- Create partitions from earliest legacy month through current month + 3
    FOR m IN
        SELECT generate_series(
            earliest,
            (date_trunc('month', now()) + interval '3 months')::date,
            interval '1 month'
        )::date
    LOOP
        PERFORM create_billing_partition(m);
    END LOOP;
END;
$$;

-- ========================================
-- STEP 8: Migrate existing data
-- ========================================
-- Copy all rows from the legacy table into the new partitioned table.
-- The rows are routed to the correct partition automatically by PostgreSQL.
INSERT INTO billing_events (
    id, tenant_id, operation, resource_id,
    tokens_in, tokens_out, gpu_seconds, cost_usd,
    metadata, created_at
)
SELECT
    id, tenant_id, operation, resource_id,
    tokens_in, tokens_out, gpu_seconds, cost_usd,
    metadata, created_at
FROM billing_events_legacy;

-- ========================================
-- STEP 9: Drop legacy table
-- ========================================
DROP TABLE billing_events_legacy;

COMMIT;

-- ========================================
-- PRODUCTION NOTES
-- ========================================
-- Automatic partition maintenance is NOT handled by this migration.
-- In production, set up one of the following:
--
-- Option A: pg_partman (recommended)
--   SELECT partman.create_parent(
--       p_parent_table := 'public.billing_events',
--       p_control := 'created_at',
--       p_type := 'range',
--       p_interval := '1 month',
--       p_premake := 3
--   );
--
-- Option B: pg_cron
--   SELECT cron.schedule(
--       'create-billing-partitions',
--       '0 0 1 * *',   -- 1st of every month at midnight
--       $$SELECT create_billing_partition(
--           (date_trunc('month', now()) + interval '3 months')::date
--       )$$
--   );
--
-- Either approach ensures future partitions exist before data arrives.
-- Without this, INSERTs into months without a partition will fail with:
--   ERROR: no partition of relation "billing_events" found for row
