-- Super-admins can suspend an account. A disabled account cannot sign in and
-- its keys stop authenticating; its data is kept so it can be re-enabled.
ALTER TABLE accounts
    ADD COLUMN disabled_at TIMESTAMPTZ,
    ADD COLUMN disabled_reason TEXT CHECK (length(disabled_reason) <= 500);
