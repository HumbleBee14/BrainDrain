-- key_prefix stores "pl_sk_" + 8 chars = 14 chars, but the column was
-- VARCHAR(10) — every API key INSERT failed. Widen with headroom.

ALTER TABLE api_keys ALTER COLUMN key_prefix TYPE VARCHAR(20);
