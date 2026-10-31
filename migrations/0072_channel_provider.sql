-- Vendor preset a channel was created from (see src/providers.rs). NULL for
-- custom channels; the preset only fills defaults, so no constraint ties it to
-- the channel's protocol or base URL.
ALTER TABLE channels ADD COLUMN provider TEXT NULL;
