ALTER TABLE channels
    ADD COLUMN IF NOT EXISTS models JSONB NOT NULL DEFAULT '[]'::jsonb,
    ADD COLUMN IF NOT EXISTS model_mapping JSONB NOT NULL DEFAULT '{}'::jsonb,
    ADD COLUMN IF NOT EXISTS model_mode TEXT NOT NULL DEFAULT 'allowlist';

UPDATE channels
SET model_mode = 'allowlist'
WHERE model_mode IS NULL OR model_mode NOT IN ('allowlist', 'passthrough');
