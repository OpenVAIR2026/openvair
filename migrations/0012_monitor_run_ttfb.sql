ALTER TABLE channel_monitor_runs
    ADD COLUMN IF NOT EXISTS ttfb_ms INTEGER;
