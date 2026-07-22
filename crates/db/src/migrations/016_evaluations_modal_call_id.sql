-- crates/db/src/migrations/016_evaluations_modal_call_id.sql
-- Durable reservation id for cloud GPU (Modal) evaluation runs.
-- Mirrors 015 for the evaluation path: written before the worker polls the
-- remote eval job, so an activity retry or worker restart reconnects to the
-- in-flight FunctionCall instead of launching a duplicate GPU run.
-- Nullable: local-provider evaluations leave it NULL.

ALTER TABLE evaluations ADD COLUMN IF NOT EXISTS modal_call_id TEXT;
