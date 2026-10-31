CREATE TABLE email_changes (
    user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    id UUID NOT NULL UNIQUE,
    session_hash TEXT NOT NULL,
    current_email TEXT NOT NULL,
    new_email TEXT NOT NULL,
    code_hash TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts BETWEEN 0 AND 5),
    ready BOOLEAN NOT NULL DEFAULT false,
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX email_changes_expiry_idx ON email_changes(expires_at);
