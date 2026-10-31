-- Move per-account limits to a single system-wide setting so the super-admin
-- can adjust them once from the platform UI instead of per-account columns.
INSERT INTO system_settings(key, value_ciphertext)
VALUES (
    'account_limits',
    '{"channel_limit":20,"upstream_key_limit":50,"virtual_key_limit":50,"member_limit":10}'
)
ON CONFLICT (key) DO NOTHING;

ALTER TABLE accounts
    DROP COLUMN IF EXISTS plan,
    DROP COLUMN IF EXISTS member_limit,
    DROP COLUMN IF EXISTS channel_limit,
    DROP COLUMN IF EXISTS upstream_key_limit,
    DROP COLUMN IF EXISTS virtual_key_limit;
