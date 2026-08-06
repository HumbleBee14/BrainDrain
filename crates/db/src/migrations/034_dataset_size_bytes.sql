-- Stored size of a dataset's JSONL objects, so datasets count toward the
-- tenant's storage allowance like documents, adapters and exports do.
--
-- Covers every object the build writes under the dataset's key: the train
-- split plus the validation and golden-holdout splits written beside it by
-- path convention. NULL means the dataset predates this column and its bytes
-- are not measured — it is not a claim that the dataset is empty.
ALTER TABLE datasets ADD COLUMN IF NOT EXISTS size_bytes BIGINT;

COMMENT ON COLUMN datasets.size_bytes IS
  'Total stored bytes of the train, val and golden JSONL objects. NULL = never measured.';
