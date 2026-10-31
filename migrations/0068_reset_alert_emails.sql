-- Existing accounts move to the new defaults as well: only availability alerts
-- stay on, and the other kinds become opt-in.
UPDATE accounts
SET alert_emails = '{"authenticity":false,"service":false,"metering":false,"availability":true}';
