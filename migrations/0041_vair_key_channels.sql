-- A virtual key from another account can be used as a channel, either by
-- pasting the key or by accepting an in-app share. Such channels store only
-- the source key id; the gateway dispatches internally with that key's limits.
CREATE TABLE key_shares (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    virtual_key_id UUID NOT NULL REFERENCES virtual_api_keys(id) ON DELETE CASCADE,
    owner_account_id UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    invite_email TEXT NOT NULL,
    grantee_account_id UUID REFERENCES accounts(id) ON DELETE CASCADE,
    created_by UUID REFERENCES users(id) ON DELETE SET NULL,
    accepted_by UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    accepted_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    CHECK (grantee_account_id IS NULL OR grantee_account_id <> owner_account_id)
);

-- An in-app share binds one key to one recipient at a time.
CREATE UNIQUE INDEX key_shares_active_key_idx
    ON key_shares(virtual_key_id) WHERE revoked_at IS NULL;
CREATE INDEX key_shares_pending_email_idx
    ON key_shares(invite_email) WHERE revoked_at IS NULL AND accepted_at IS NULL;

ALTER TABLE channels
    ADD COLUMN kind TEXT NOT NULL DEFAULT 'upstream'
        CHECK (kind IN ('upstream', 'vair_key')),
    ADD COLUMN source_key_id UUID REFERENCES virtual_api_keys(id) ON DELETE SET NULL,
    ADD COLUMN key_share_id UUID REFERENCES key_shares(id) ON DELETE SET NULL;

CREATE UNIQUE INDEX channels_account_source_key_idx
    ON channels(account_id, source_key_id) WHERE source_key_id IS NOT NULL;
CREATE INDEX channels_source_key_idx
    ON channels(source_key_id) WHERE source_key_id IS NOT NULL;

-- VAIR key channels accept every gateway protocol.
ALTER TABLE channels DROP CONSTRAINT channels_protocol_check;
ALTER TABLE channels ADD CONSTRAINT channels_protocol_check
    CHECK (protocol IN ('openai', 'anthropic', 'gemini')
           OR (protocol = 'any' AND kind = 'vair_key'));

-- The caller's usage row points at the source key's usage row so platform-wide
-- statistics can count each relayed request once. Monitoring probes through a
-- VAIR key channel are recorded on the source key so they count toward its caps.
ALTER TABLE usage_events
    ADD COLUMN relayed_event_id UUID REFERENCES usage_events(id) ON DELETE SET NULL,
    ADD COLUMN source TEXT NOT NULL DEFAULT 'gateway'
        CHECK (source IN ('gateway', 'monitor'));
