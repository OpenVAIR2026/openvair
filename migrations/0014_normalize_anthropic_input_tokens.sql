-- Anthropic reports uncached input separately from cache reads. Bring existing
-- records in line with the normalized input count used for new requests.
UPDATE usage_events
SET input_tokens = input_tokens + cached_tokens
WHERE protocol = 'anthropic'
  AND input_tokens IS NOT NULL
  AND cached_tokens IS NOT NULL
  AND cached_tokens > 0;
