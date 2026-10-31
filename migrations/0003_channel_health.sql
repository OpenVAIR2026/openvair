CREATE TABLE IF NOT EXISTS channel_health (
    channel_id UUID PRIMARY KEY REFERENCES channels(id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'unknown',
    last_error TEXT,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    checked_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
