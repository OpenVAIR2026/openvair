-- One pending password reset per user. The code is stored as a hash.
CREATE TABLE password_resets (
    user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    id UUID NOT NULL UNIQUE,
    code_hash TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts BETWEEN 0 AND 5),
    expires_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX password_resets_expiry_idx ON password_resets(expires_at);

-- Lets users recognize and sign out their other devices. Existing sessions
-- have no device information and show as unknown.
ALTER TABLE sessions
    ADD COLUMN user_agent TEXT CHECK (length(user_agent) <= 256),
    ADD COLUMN last_seen_at TIMESTAMPTZ;
