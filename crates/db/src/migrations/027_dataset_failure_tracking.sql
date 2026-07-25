-- Refine reserves a dataset row up front so generation is visible while it runs
-- and recordable when it fails. Before this, the row was only created by the
-- final build_dataset step, so a failed run left no trace anywhere.
ALTER TABLE datasets ADD COLUMN IF NOT EXISTS error TEXT;

CREATE INDEX IF NOT EXISTS idx_datasets_project_status
    ON datasets(project_id, status);
