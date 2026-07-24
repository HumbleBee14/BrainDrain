-- Rust decodes cost columns as f64, which sqlx cannot read from NUMERIC —
-- inserts coerced silently but every SELECT failed. Align storage with code.

ALTER TABLE training_jobs ALTER COLUMN cost_estimate TYPE DOUBLE PRECISION;
ALTER TABLE training_jobs ALTER COLUMN actual_cost TYPE DOUBLE PRECISION;
ALTER TABLE billing_events ALTER COLUMN cost_usd TYPE DOUBLE PRECISION;
ALTER TABLE billing_outbox ALTER COLUMN cost_usd TYPE DOUBLE PRECISION;
