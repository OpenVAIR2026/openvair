-- Consecutive failures (checks or real requests) before a channel is judged
-- unavailable, and whether that moves it to the end of routing or only alerts.
ALTER TABLE channel_monitor_configs
    ADD COLUMN availability_failure_threshold INTEGER NOT NULL DEFAULT 2
        CHECK (availability_failure_threshold BETWEEN 1 AND 5),
    ADD COLUMN availability_route_policy TEXT NOT NULL DEFAULT 'exclude'
        CHECK (availability_route_policy IN ('exclude', 'observe'));

-- A streaming request that has produced no output by then fails over to the
-- next channel. The API accepts 5-300 seconds; tests use shorter values.
ALTER TABLE channels
    ADD COLUMN first_token_timeout_seconds INTEGER NOT NULL DEFAULT 40
        CHECK (first_token_timeout_seconds BETWEEN 1 AND 600);
