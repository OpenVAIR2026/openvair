-- Key groups share one channel and model scope. The scope is copied onto each
-- member key whenever the group or its membership changes, so the gateway keeps
-- reading only the key. Limits stay on each key.
CREATE TABLE key_groups (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 120),
    channel_mode TEXT NOT NULL DEFAULT 'all' CHECK (channel_mode IN ('all', 'selected')),
    model_mode TEXT NOT NULL DEFAULT 'all' CHECK (model_mode IN ('all', 'allowlist')),
    allowed_models JSONB NOT NULL DEFAULT '[]'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (id, account_id)
);
CREATE UNIQUE INDEX key_groups_account_name_idx ON key_groups(account_id, lower(name));

CREATE TABLE key_group_channels (
    group_id UUID NOT NULL REFERENCES key_groups(id) ON DELETE CASCADE,
    channel_id UUID NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    PRIMARY KEY (group_id, channel_id)
);
CREATE INDEX key_group_channels_channel_idx ON key_group_channels(channel_id);

-- A key can only join a group of its own account. Deleting a group first
-- detaches its keys, which keep their copied scope.
ALTER TABLE virtual_api_keys ADD COLUMN group_id UUID;
ALTER TABLE virtual_api_keys ADD CONSTRAINT virtual_api_keys_group_fk
    FOREIGN KEY (group_id, account_id) REFERENCES key_groups(id, account_id);
CREATE INDEX virtual_api_keys_group_idx ON virtual_api_keys(group_id) WHERE group_id IS NOT NULL;
