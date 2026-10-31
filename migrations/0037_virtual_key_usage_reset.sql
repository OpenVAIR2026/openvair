-- Resets move the start of a key's limit windows; usage events stay intact.
ALTER TABLE virtual_api_keys
    ADD COLUMN usage_reset_daily_at TIMESTAMPTZ,
    ADD COLUMN usage_reset_total_at TIMESTAMPTZ;
