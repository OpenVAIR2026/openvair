ALTER TABLE organizations
    ADD COLUMN api_key_prefix TEXT NOT NULL DEFAULT 'mr_live';

ALTER TABLE organizations
    ADD CONSTRAINT organizations_api_key_prefix_format
    CHECK (api_key_prefix ~ '^[A-Za-z0-9][A-Za-z0-9_-]{0,31}$');
