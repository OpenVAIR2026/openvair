-- The gateway sums a key's spend and tokens on every request to enforce its
-- limits. This index answers that from the index alone instead of combining
-- two single-column indexes and visiting the table. It is built plainly, not
-- CONCURRENTLY: migrations run before the new version serves traffic, and an
-- interrupted concurrent build would leave an invalid index that makes the
-- next start fail.
CREATE INDEX usage_key_limits_idx ON usage_events(account_id, virtual_key_id, created_at)
  INCLUDE (cost_usd, input_tokens, output_tokens, status, in_flight_until);
