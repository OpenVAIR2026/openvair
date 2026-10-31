-- A one-time link hands a key to another account, which adds it as a VAIR key
-- channel without seeing the secret. The link only creates the channel: the
-- channel depends on source_key_id alone, exactly like a pasted key, so
-- revoking the key is still the only way to take it back. Only the code's hash
-- is stored.
CREATE TABLE key_share_links (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    virtual_key_id UUID NOT NULL REFERENCES virtual_api_keys(id) ON DELETE CASCADE,
    account_id UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    code_hash TEXT NOT NULL UNIQUE,
    created_by UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    accepted_account_id UUID REFERENCES accounts(id) ON DELETE SET NULL,
    accepted_at TIMESTAMPTZ,
    cancelled_at TIMESTAMPTZ,
    CHECK (accepted_account_id IS NULL OR accepted_account_id <> account_id)
);

-- A key has at most one open link; issuing a new one cancels the old.
CREATE UNIQUE INDEX key_share_links_open_idx
    ON key_share_links(virtual_key_id) WHERE accepted_at IS NULL AND cancelled_at IS NULL;
CREATE INDEX key_share_links_accepted_idx
    ON key_share_links(virtual_key_id, accepted_account_id) WHERE accepted_at IS NOT NULL;
