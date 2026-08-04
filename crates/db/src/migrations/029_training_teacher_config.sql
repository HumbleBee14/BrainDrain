-- Distill-mode training jobs record which external teacher model generated
-- their training data, so provenance and future teacher-dependent stages can
-- reconstruct the exact teacher. Nullable: every other mode leaves it NULL.
ALTER TABLE training_jobs ADD COLUMN IF NOT EXISTS teacher_config JSONB;

COMMENT ON COLUMN training_jobs.teacher_config IS
  'Distill mode: teacher endpoint/model provenance. api_key value is SecretCipher-encrypted (enc:v1).';
