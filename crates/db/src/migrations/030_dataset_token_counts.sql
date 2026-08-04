-- Token counts for a dataset, used to estimate and admit teacher logprob
-- extraction before any GPU is provisioned.
--
-- Counts are a property of a (dataset, tokenizer) pair, not of the dataset
-- alone: the same records tokenize to different totals under different
-- tokenizers. The hash column records which tokenizer produced these numbers so
-- a run under a different one recomputes instead of trusting them.
ALTER TABLE datasets ADD COLUMN IF NOT EXISTS prompt_tokens BIGINT;
ALTER TABLE datasets ADD COLUMN IF NOT EXISTS completion_tokens BIGINT;
ALTER TABLE datasets ADD COLUMN IF NOT EXISTS scored_completion_tokens BIGINT;
ALTER TABLE datasets ADD COLUMN IF NOT EXISTS token_count_tokenizer_hash VARCHAR(64);

COMMENT ON COLUMN datasets.scored_completion_tokens IS
  'Completion positions a teacher would score. Valid only for token_count_tokenizer_hash.';
COMMENT ON COLUMN datasets.token_count_tokenizer_hash IS
  'Combined tokenizer-artifact hash the token counts were measured under. NULL = never measured.';
