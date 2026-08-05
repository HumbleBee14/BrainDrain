-- crates/db/src/migrations/033_model_exports_modal_call_id.sql
-- Durable reservation id for cloud GPU (Modal) export runs.
-- Mirrors 015/016 for the GGUF export path: written before the worker polls
-- the remote export job, so an activity retry or worker restart reconnects to
-- the in-flight FunctionCall instead of launching a duplicate run.
-- Nullable: local-provider exports leave it NULL.

ALTER TABLE model_exports ADD COLUMN IF NOT EXISTS modal_call_id TEXT;
