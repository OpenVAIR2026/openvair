-- New accounts issue keys like vair_...; existing accounts keep their prefix and keys.
ALTER TABLE accounts ALTER COLUMN api_key_prefix SET DEFAULT 'vair';
