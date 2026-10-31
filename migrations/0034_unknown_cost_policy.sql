ALTER TABLE virtual_api_keys
    ADD COLUMN unknown_cost_policy TEXT NOT NULL DEFAULT 'allow'
        CHECK (unknown_cost_policy IN ('allow', 'block'));

UPDATE virtual_api_keys SET unknown_cost_policy = 'block'
WHERE cost_limit_daily_usd IS NOT NULL;

-- A deadline also releases in-flight accounting after an interrupted process.
ALTER TABLE usage_events ADD COLUMN in_flight_until TIMESTAMPTZ;
