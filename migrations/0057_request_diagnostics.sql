-- NULL means no diagnostics were collected (including historical requests).
-- Contains timings and channel IDs only, never request bodies or credentials.
ALTER TABLE usage_events ADD COLUMN request_diagnostics JSONB;
