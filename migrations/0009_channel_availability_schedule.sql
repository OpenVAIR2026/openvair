ALTER TABLE channel_monitor_configs
    ADD COLUMN IF NOT EXISTS availability_interval_seconds INTEGER NOT NULL DEFAULT 1800,
    ADD COLUMN IF NOT EXISTS next_availability_at TIMESTAMPTZ;

