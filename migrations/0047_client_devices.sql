-- VAIR Desktop signs in with a device authorization grant: the app shows a
-- short user code, the user approves it in a signed-in browser, and the app
-- exchanges its device code for a client token. Only hashes are stored.
CREATE TABLE client_device_authorizations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    device_code_hash TEXT NOT NULL UNIQUE,
    user_code TEXT NOT NULL UNIQUE,
    client_name TEXT NOT NULL CHECK (length(client_name) BETWEEN 1 AND 120),
    user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending','approved','denied','consumed')),
    last_polled_at TIMESTAMPTZ,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX client_device_authorizations_expiry_idx ON client_device_authorizations(expires_at);

-- Client tokens are narrower than browser sessions: they can list keys and
-- reveal one, nothing else. They slide forward while in use.
CREATE TABLE client_tokens (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    client_name TEXT NOT NULL CHECK (length(client_name) BETWEEN 1 AND 120),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at TIMESTAMPTZ,
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ
);
CREATE INDEX client_tokens_user_idx ON client_tokens(user_id);
