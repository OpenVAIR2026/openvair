-- Keys may be limited to selected channels; existing keys keep routing to all.
ALTER TABLE virtual_api_keys
    ADD COLUMN channel_mode TEXT NOT NULL DEFAULT 'all'
        CHECK (channel_mode IN ('all', 'selected'));

-- Deleting a channel or key removes the link; a selected key left without
-- channels routes nowhere rather than falling back to every channel.
CREATE TABLE virtual_key_channels (
    virtual_key_id UUID NOT NULL REFERENCES virtual_api_keys(id) ON DELETE CASCADE,
    channel_id UUID NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    PRIMARY KEY (virtual_key_id, channel_id)
);
CREATE INDEX virtual_key_channels_channel_idx ON virtual_key_channels(channel_id);
