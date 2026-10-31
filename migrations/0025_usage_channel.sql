-- Record which upstream channel served each request so usage analysis can
-- break traffic, cache rate and errors down per channel.
ALTER TABLE usage_events
    ADD COLUMN IF NOT EXISTS channel_id UUID REFERENCES channels(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS usage_account_channel_created_idx
    ON usage_events(account_id, channel_id, created_at DESC);
