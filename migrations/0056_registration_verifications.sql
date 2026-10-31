-- One pending sign-up code per email address. The code is stored as a hash and
-- is only issued for addresses that are not registered yet.
CREATE TABLE registration_verifications (
    email TEXT PRIMARY KEY,
    id UUID NOT NULL UNIQUE,
    code_hash TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts BETWEEN 0 AND 5),
    expires_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX registration_verifications_expiry_idx ON registration_verifications(expires_at);
