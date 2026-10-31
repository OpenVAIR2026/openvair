-- History of a key's total cost limit. Each increase, direct change and full
-- usage reset leaves one row; the owner, the key holder and accounts that use
-- the key as a channel all read the same rows. Notes are visible to all of them.
CREATE TABLE key_quota_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    virtual_key_id UUID NOT NULL REFERENCES virtual_api_keys(id) ON DELETE CASCADE,
    account_id UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('opening','increase','set','reset')),
    amount_usd DOUBLE PRECISION CHECK (amount_usd IS NULL OR amount_usd > 0),
    limit_before_usd DOUBLE PRECISION,
    limit_after_usd DOUBLE PRECISION,
    used_before_usd DOUBLE PRECISION,
    note TEXT CHECK (note IS NULL OR char_length(note) BETWEEN 1 AND 200),
    created_by UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX key_quota_events_key_idx ON key_quota_events(virtual_key_id, created_at DESC);

-- Earlier changes were not recorded; start each capped key's history at its
-- current limit so later rows have a known starting point.
INSERT INTO key_quota_events(virtual_key_id,account_id,kind,limit_after_usd)
SELECT id,account_id,'opening',cost_limit_total_usd FROM virtual_api_keys
WHERE revoked_at IS NULL AND cost_limit_total_usd IS NOT NULL;
