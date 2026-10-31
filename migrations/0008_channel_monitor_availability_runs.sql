-- Keep the historical monitor table useful for both the existing quality
-- checks and the availability checks performed by the gateway health loop.
ALTER TABLE channel_monitor_runs
    DROP CONSTRAINT IF EXISTS channel_monitor_runs_kind_check,
    DROP CONSTRAINT IF EXISTS channel_monitor_runs_status_check;

ALTER TABLE channel_monitor_runs
    ADD CONSTRAINT channel_monitor_runs_kind_check
        CHECK (kind IN ('availability', 'quality')),
    ADD CONSTRAINT channel_monitor_runs_status_check
        CHECK (status IN ('healthy', 'degraded', 'unhealthy', 'unknown'));

