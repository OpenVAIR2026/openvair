-- 40 seconds failed over reasoning models that legitimately think longer.
-- Channels still on the old default move with it; other values were chosen.
ALTER TABLE channels ALTER COLUMN first_token_timeout_seconds SET DEFAULT 100;
UPDATE channels SET first_token_timeout_seconds = 100 WHERE first_token_timeout_seconds = 40;
