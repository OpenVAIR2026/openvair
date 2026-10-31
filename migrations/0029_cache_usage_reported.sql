-- Preserve the distinction between unreported cache usage and an explicit zero.
-- NULL is reserved for older records whose reporting status cannot be recovered.
ALTER TABLE usage_events ADD COLUMN cache_usage_reported BOOLEAN;

-- A positive historical count is evidence that the upstream reported cache reads.
-- Historical zero/NULL values must not be guessed or rewritten.
UPDATE usage_events SET cache_usage_reported = true WHERE cached_tokens > 0;
