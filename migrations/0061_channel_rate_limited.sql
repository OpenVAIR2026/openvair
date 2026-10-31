-- A rate-limited check, or one refused for its test model, reached the
-- upstream: shown, but neither counted as a failure nor keeping the channel
-- out of routing. Refused checks remember whether the key ('credential') or
-- the balance ('quota') needs attention.
ALTER TABLE channel_health
    ADD COLUMN last_error_kind TEXT CHECK (last_error_kind IN ('credential', 'quota'));

ALTER TABLE channel_monitor_runs
    DROP CONSTRAINT channel_monitor_runs_status_check,
    ADD CONSTRAINT channel_monitor_runs_status_check
        CHECK (status IN ('healthy', 'degraded', 'unhealthy', 'rate_limited', 'model_unavailable', 'unknown'));
