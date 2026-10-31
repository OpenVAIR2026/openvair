-- Retire body retention while keeping legacy storage compatible with rollback.
-- Existing encrypted logs remain subject to the hourly expiry cleanup.
UPDATE organizations SET request_logging_enabled = false;
