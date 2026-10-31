-- Lifetime spend cap; counts priced usage since the key was created.
ALTER TABLE virtual_api_keys ADD COLUMN cost_limit_total_usd DOUBLE PRECISION;
