-- The Rust models decode cost columns as f64 (FLOAT8), but the schema declared
-- them DECIMAL/NUMERIC. INSERTs worked (implicit float8->numeric assignment
-- cast) while every SELECT decode failed — training-job creation and billing
-- row reads returned 500s. Align the storage type with the code.
-- (Aggregate queries already cast ::FLOAT8 and keep working.)

ALTER TABLE training_jobs ALTER COLUMN cost_estimate TYPE DOUBLE PRECISION;
ALTER TABLE training_jobs ALTER COLUMN actual_cost TYPE DOUBLE PRECISION;
-- Partitioned parent: applies to all existing and future partitions.
ALTER TABLE billing_events ALTER COLUMN cost_usd TYPE DOUBLE PRECISION;
ALTER TABLE billing_outbox ALTER COLUMN cost_usd TYPE DOUBLE PRECISION;
