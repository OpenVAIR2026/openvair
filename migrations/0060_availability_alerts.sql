-- Availability emails go out only for sustained or credential failures, at
-- most once a day per channel, with a recovery email only after an alert.
ALTER TABLE channel_health
    ADD COLUMN unhealthy_since TIMESTAMPTZ,
    ADD COLUMN alert_open BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN last_alert_at TIMESTAMPTZ;

-- Channels already unavailable were alerted under the old rules.
UPDATE channel_health
SET unhealthy_since = now(), alert_open = true, last_alert_at = now()
WHERE status = 'unhealthy';
