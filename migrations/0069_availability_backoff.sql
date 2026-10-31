-- Checks after a failure back off by how long the channel has been failing,
-- so a burst of failed requests cannot push the next check out at once.
ALTER TABLE channel_health ADD COLUMN failing_since TIMESTAMPTZ;

UPDATE channel_health
SET failing_since = COALESCE(unhealthy_since, checked_at, now())
WHERE consecutive_failures > 0;
