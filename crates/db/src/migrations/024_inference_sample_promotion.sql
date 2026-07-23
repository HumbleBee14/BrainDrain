-- Data flywheel stage 2: promoted samples become training dataset records.
-- promoted_at guards against double-promotion (duplicate training rows).

ALTER TABLE inference_samples ADD COLUMN IF NOT EXISTS promoted_at TIMESTAMPTZ;
