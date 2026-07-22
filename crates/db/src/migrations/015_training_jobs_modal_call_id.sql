-- crates/db/src/migrations/015_training_jobs_modal_call_id.sql
-- Durable reservation id for cloud GPU (Modal) training runs.
-- Written before the worker polls the remote job, so an activity retry or
-- worker restart reconnects to the in-flight FunctionCall instead of
-- launching a duplicate GPU run. Nullable: local-provider jobs leave it NULL.

ALTER TABLE training_jobs ADD COLUMN IF NOT EXISTS modal_call_id TEXT;
