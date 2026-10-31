ALTER TABLE organizations
    ADD COLUMN IF NOT EXISTS request_logging_enabled BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN IF NOT EXISTS request_retention_days INTEGER NOT NULL DEFAULT 7;
