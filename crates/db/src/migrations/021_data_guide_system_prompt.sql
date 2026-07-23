-- Optional per-guide system prompt. Baked into every training example's
-- `system` role at dataset build time and reused as the serving default, so a
-- fine-tuned model is served under the same system prompt it was trained on.
-- Empty string preserves the prior neutral-default behavior.
ALTER TABLE data_guides
    ADD COLUMN IF NOT EXISTS system_prompt TEXT NOT NULL DEFAULT '';
