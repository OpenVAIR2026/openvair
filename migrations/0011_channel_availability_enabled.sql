ALTER TABLE channel_monitor_configs
    ADD COLUMN IF NOT EXISTS availability_enabled BOOLEAN NOT NULL DEFAULT true;
