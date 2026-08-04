-- Stage 2 distillation runs an optional teacher logprob extraction on our own
-- metered GPU before training. Extraction is a first-class billable workload,
-- not "like training" by analogy: the existing training_jobs.status/cost
-- columns track the training run itself, so extraction needs its own state
-- to avoid conflating "training failed" with "extraction failed" (and to keep
-- a cancelled extraction distinguishable from a cancelled training run in
-- billing). Nullable throughout: every job that never requests a fidelity
-- upgrade leaves all three NULL.
ALTER TABLE training_jobs ADD COLUMN IF NOT EXISTS teacher_extraction_status VARCHAR(50);
ALTER TABLE training_jobs ADD COLUMN IF NOT EXISTS teacher_extraction_modal_call_id TEXT;
ALTER TABLE training_jobs ADD COLUMN IF NOT EXISTS teacher_extraction_cost DOUBLE PRECISION;

COMMENT ON COLUMN training_jobs.teacher_extraction_status IS
  'Lifecycle state of the teacher logprob extraction run. NULL = no fidelity upgrade requested.';
COMMENT ON COLUMN training_jobs.teacher_extraction_modal_call_id IS
  'Reservation id for the extraction FunctionCall, mirroring modal_call_id for training so a worker restart reconnects instead of relaunching the GPU.';
COMMENT ON COLUMN training_jobs.teacher_extraction_cost IS
  'Finalized teacher-GPU cost for extraction, billed separately from actual_cost (the training run''s own cost).';
