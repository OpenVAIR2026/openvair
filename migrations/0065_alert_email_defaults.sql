-- New accounts only get availability alerts by default; the other kinds are
-- opt-in. Settings already saved by existing accounts are left unchanged.
ALTER TABLE accounts
    ALTER COLUMN alert_emails SET DEFAULT
        '{"authenticity":false,"service":false,"metering":false,"availability":true}';
