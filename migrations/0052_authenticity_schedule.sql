-- Scheduled authenticity sampling: authenticity monitors keep a claimed
-- model, sample a few probes per run, and judge a rolling window of samples.
-- The window verdict may steer routing and triggers alerts; deep checks stay
-- separate so the two can be compared.
ALTER TABLE quality_monitors
    ADD COLUMN claimed_model TEXT,
    ADD COLUMN samples_per_run INTEGER NOT NULL DEFAULT 2 CHECK (samples_per_run BETWEEN 1 AND 8),
    ADD COLUMN window_days INTEGER NOT NULL DEFAULT 7 CHECK (window_days BETWEEN 1 AND 30),
    ADD COLUMN window_verdict TEXT,
    ADD COLUMN window_evidence JSONB,
    ADD COLUMN window_checked_at TIMESTAMPTZ,
    ADD COLUMN window_valid_until TIMESTAMPTZ,
    ADD COLUMN alerted_verdict TEXT;

-- The interval check allowed 15 minutes for service checks; authenticity
-- monitors default to six hours to keep sampling cheap.
UPDATE quality_monitors SET interval_seconds=21600 WHERE kind='authenticity';

-- Samples remember what they were judged against, so a window only mixes
-- samples of the same claim and baseline.
ALTER TABLE authenticity_samples
    ADD COLUMN claimed_model TEXT,
    ADD COLUMN baseline_version_id UUID;
UPDATE authenticity_samples s SET claimed_model=j.claimed_model,
    baseline_version_id=NULLIF(j.options->>'baseline_version_id','')::uuid
FROM quality_monitor_jobs j WHERE j.id=s.job_id;
CREATE INDEX authenticity_samples_window_idx ON authenticity_samples(monitor_id,claimed_model,baseline_version_id,created_at DESC);
