-- Data Studio runs recorded `status = 'failed'` with no reason, so the UI could
-- only show that something broke.
ALTER TABLE data_guides ADD COLUMN IF NOT EXISTS error TEXT;
