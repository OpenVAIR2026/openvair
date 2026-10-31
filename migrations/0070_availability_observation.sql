-- The status is the availability verdict that routing reads; a check that
-- says nothing about availability (a rate limit, a refused test model, or one
-- that never reached the upstream) is kept apart so it cannot clear the verdict.
ALTER TABLE channel_health
    ADD COLUMN last_observation TEXT CHECK (last_observation IN ('rate_limited', 'model_unavailable', 'unknown')),
    ADD COLUMN last_failure_at TIMESTAMPTZ;

UPDATE channel_health
SET last_failure_at = COALESCE(checked_at, now())
WHERE consecutive_failures > 0;

-- Such checks used to replace the verdict, so the verdict is not known.
UPDATE channel_health
SET last_observation = status, status = 'unknown'
WHERE status IN ('rate_limited', 'model_unavailable');
