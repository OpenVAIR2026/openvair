CREATE TABLE system_settings (
    key TEXT PRIMARY KEY,
    value_ciphertext TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
