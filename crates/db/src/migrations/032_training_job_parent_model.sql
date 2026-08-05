-- An on-policy improve pass sharpens a model that already exists, so the run has
-- a parent: the model whose own answers the teacher graded. Recording it is what
-- makes "matched 87% before, 94% after" answerable — the two parity reports are
-- comparable only because they were measured on the same dataset's golden set.
--
-- Nullable, and NULL for every run that is not an improve pass, which is every
-- run of every mode that existed before on-policy distillation.
--
-- ON DELETE SET NULL rather than CASCADE: deleting an old model must not delete
-- the history of the better model that replaced it.
ALTER TABLE training_jobs ADD COLUMN IF NOT EXISTS parent_model_id UUID
  REFERENCES models(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_training_jobs_parent_model
  ON training_jobs(tenant_id, parent_model_id)
  WHERE parent_model_id IS NOT NULL;

COMMENT ON COLUMN training_jobs.parent_model_id IS
  'Model this run improves on (on-policy distillation). NULL = not an improve pass.';
