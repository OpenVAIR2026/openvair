ALTER TABLE channel_health
    ADD COLUMN IF NOT EXISTS quality_status TEXT NOT NULL DEFAULT 'unknown',
    ADD COLUMN IF NOT EXISTS quality_last_observation TEXT NOT NULL DEFAULT 'unknown',
    ADD COLUMN IF NOT EXISTS quality_score NUMERIC(5,2),
    ADD COLUMN IF NOT EXISTS quality_error TEXT,
    ADD COLUMN IF NOT EXISTS quality_checked_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS quality_consecutive_failures INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS quality_valid_until TIMESTAMPTZ;

CREATE TABLE IF NOT EXISTS channel_monitor_configs (
    channel_id UUID PRIMARY KEY REFERENCES channels(id) ON DELETE CASCADE,
    quality_enabled BOOLEAN NOT NULL DEFAULT false,
    quality_model TEXT,
    quality_interval_seconds INTEGER NOT NULL DEFAULT 3600,
    quality_timeout_ms INTEGER NOT NULL DEFAULT 60000,
    quality_min_score NUMERIC(5,2) NOT NULL DEFAULT 75,
    quality_failure_threshold INTEGER NOT NULL DEFAULT 2,
    quality_daily_budget_micros BIGINT NOT NULL DEFAULT 0,
    suite_version TEXT NOT NULL DEFAULT 'v1',
    next_quality_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO channel_monitor_configs(channel_id)
SELECT id FROM channels
ON CONFLICT (channel_id) DO NOTHING;

CREATE TABLE IF NOT EXISTS channel_monitor_runs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    channel_id UUID NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('quality')),
    model TEXT,
    status TEXT NOT NULL CHECK (status IN ('healthy', 'degraded', 'unknown')),
    score NUMERIC(5,2),
    latency_ms INTEGER,
    status_code INTEGER,
    error_code TEXT,
    details JSONB NOT NULL DEFAULT '{}'::jsonb,
    started_at TIMESTAMPTZ NOT NULL,
    finished_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS channel_monitor_runs_lookup_idx
    ON channel_monitor_runs(channel_id, kind, created_at DESC);
