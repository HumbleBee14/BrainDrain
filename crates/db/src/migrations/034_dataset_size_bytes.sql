-- Stored size of a dataset's JSONL objects (train + val + golden), so datasets
-- count toward the tenant's storage allowance. NULL = never measured.
ALTER TABLE datasets ADD COLUMN IF NOT EXISTS size_bytes BIGINT;

COMMENT ON COLUMN datasets.size_bytes IS
  'Total stored bytes of the train, val and golden JSONL objects. NULL = never measured.';
