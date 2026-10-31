-- Real traffic feeds output performance and passive metering: whether a
-- request streamed, when its first output arrived, and how many characters of
-- text it sent. ASCII and other characters (mostly CJK) tokenize very
-- differently, so they are counted apart. Only counts are kept, never content.
ALTER TABLE usage_events
    ADD COLUMN stream BOOLEAN,
    ADD COLUMN first_token_ms INTEGER,
    ADD COLUMN input_ascii_chars INTEGER,
    ADD COLUMN input_other_chars INTEGER;
CREATE INDEX usage_events_metering_idx ON usage_events(created_at)
    WHERE input_ascii_chars IS NOT NULL;

-- The latest passive metering fit of each channel and upstream model over the
-- last 24 hours. Rows disappear when there is no longer enough traffic.
CREATE TABLE channel_passive_stats (
    channel_id UUID NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    upstream_model TEXT NOT NULL,
    account_id UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    samples INTEGER NOT NULL,
    tokens_per_ascii DOUBLE PRECISION,
    tokens_per_other DOUBLE PRECISION,
    overhead DOUBLE PRECISION NOT NULL,
    warnings JSONB NOT NULL DEFAULT '[]',
    -- Warning kinds already emailed; cleared when the warnings go away.
    alerted JSONB NOT NULL DEFAULT '[]',
    computed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (channel_id, upstream_model)
);
CREATE INDEX channel_passive_stats_account_idx ON channel_passive_stats(account_id);

-- Which alerts are emailed to the account. Every kind can be turned off.
ALTER TABLE accounts
    ADD COLUMN alert_emails JSONB NOT NULL
        DEFAULT '{"authenticity":true,"service":true,"metering":true}';

-- Service checks run early when real traffic looks off, and long-context
-- checks run on demand; both share the monitor job queue.
ALTER TABLE quality_monitor_jobs DROP CONSTRAINT quality_monitor_jobs_source_check;
ALTER TABLE quality_monitor_jobs ADD CONSTRAINT quality_monitor_jobs_source_check
    CHECK (source IN ('manual','scheduled','create','deep','triggered','long_context'));
ALTER TABLE channel_monitor_runs DROP CONSTRAINT channel_monitor_runs_kind_check;
ALTER TABLE channel_monitor_runs ADD CONSTRAINT channel_monitor_runs_kind_check
    CHECK (kind IN ('availability','quality','long_context'));
ALTER TABLE quality_monitors
    ADD COLUMN long_context_max INTEGER,
    ADD COLUMN long_context_checked_at TIMESTAMPTZ;
