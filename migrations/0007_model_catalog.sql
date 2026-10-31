ALTER TABLE users
    ADD COLUMN IF NOT EXISTS is_super_admin BOOLEAN NOT NULL DEFAULT false;

CREATE TABLE IF NOT EXISTS model_catalog (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    pattern TEXT NOT NULL UNIQUE,
    label TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO model_catalog(pattern, label) VALUES
    ('gpt-*', 'GPT 系列'),
    ('claude-*', 'Claude 系列'),
    ('gemini-*', 'Gemini 系列')
ON CONFLICT (pattern) DO NOTHING;
