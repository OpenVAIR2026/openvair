-- Model authenticity checks (quality monitoring v3), stage one:
-- a secret probe prompt pool, imported model baselines, and deep checks.

-- Prompt pool. Templates are secret (relays could match public wording), so
-- the text is stored encrypted and never kept in the repository.
CREATE TABLE probe_templates (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    probe_kind TEXT NOT NULL CHECK (probe_kind IN ('fingerprint','token')),
    content_ciphertext TEXT NOT NULL,
    created_by UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    retired_at TIMESTAMPTZ
);

-- A published pool version is an immutable snapshot of enabled templates.
-- Baselines and checks are only comparable within one pool version.
CREATE TABLE probe_pool_versions (
    version INTEGER PRIMARY KEY CHECK (version > 0),
    template_ids UUID[] NOT NULL,
    published_by UUID REFERENCES users(id) ON DELETE SET NULL,
    published_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Which models have baselines and how to collect them.
CREATE TABLE model_baseline_targets (
    model TEXT PRIMARY KEY CHECK (length(btrim(model)) BETWEEN 1 AND 256 AND strpos(model,'*')=0),
    family TEXT NOT NULL CHECK (length(btrim(family)) BETWEEN 1 AND 64),
    openrouter_model TEXT NOT NULL CHECK (length(btrim(openrouter_model)) BETWEEN 1 AND 256),
    provider_only TEXT NOT NULL CHECK (length(btrim(provider_only)) BETWEEN 1 AND 64),
    target_samples INTEGER NOT NULL DEFAULT 200 CHECK (target_samples BETWEEN 20 AND 5000),
    token_samples INTEGER NOT NULL DEFAULT 50 CHECK (token_samples BETWEEN 0 AND 1000),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- A collection plan carries the secret pool, so it expires and downloads are audited.
CREATE TABLE model_baseline_plans (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    pool_version INTEGER NOT NULL REFERENCES probe_pool_versions(version),
    items JSONB NOT NULL,
    created_by UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    downloaded_at TIMESTAMPTZ
);

-- A version fixes the collection conditions (the compatibility key). Batches
-- with the same key are merged incrementally; each merge or exclusion bumps
-- the revision and recomputes stats from the included batches.
CREATE TABLE model_baseline_versions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    model TEXT NOT NULL REFERENCES model_baseline_targets(model) ON DELETE CASCADE,
    family TEXT NOT NULL,
    source TEXT NOT NULL CHECK (source IN ('openrouter')),
    provider_only TEXT NOT NULL,
    pool_version INTEGER NOT NULL REFERENCES probe_pool_versions(version),
    reasoning TEXT NOT NULL,
    range_max INTEGER NOT NULL CHECK (range_max BETWEEN 10 AND 10000),
    format_version INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft','active','retired')),
    revision INTEGER NOT NULL DEFAULT 0,
    stats JSONB NOT NULL DEFAULT '{}',
    valid_samples INTEGER NOT NULL DEFAULT 0,
    created_by UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    activated_at TIMESTAMPTZ
);
CREATE UNIQUE INDEX model_baseline_versions_active_idx ON model_baseline_versions(model) WHERE status='active';

CREATE TABLE model_baseline_batches (
    id UUID PRIMARY KEY,
    model TEXT NOT NULL,
    version_id UUID REFERENCES model_baseline_versions(id) ON DELETE CASCADE,
    plan_id UUID REFERENCES model_baseline_plans(id) ON DELETE SET NULL,
    manifest JSONB NOT NULL,
    samples_sha256 TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','merged','rejected')),
    validation JSONB NOT NULL,
    drift JSONB,
    preview JSONB,
    sample_count INTEGER NOT NULL,
    valid_count INTEGER NOT NULL,
    imported_by UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    decided_by UUID REFERENCES users(id) ON DELETE SET NULL,
    decided_at TIMESTAMPTZ,
    excluded_at TIMESTAMPTZ
);
CREATE INDEX model_baseline_batches_version_idx ON model_baseline_batches(version_id) WHERE status='merged' AND excluded_at IS NULL;

-- Only parsed integers and counts are kept, never answer text.
CREATE TABLE model_baseline_samples (
    batch_id UUID NOT NULL REFERENCES model_baseline_batches(id) ON DELETE CASCADE,
    sample_id TEXT NOT NULL,
    probe_kind TEXT NOT NULL CHECK (probe_kind IN ('fingerprint','token')),
    template_id UUID NOT NULL,
    n INTEGER,
    sample_values INTEGER[] NOT NULL DEFAULT '{}',
    input_tokens INTEGER,
    output_tokens INTEGER,
    reasoning_tokens INTEGER,
    served_provider TEXT,
    valid BOOLEAN NOT NULL,
    invalid_reason TEXT,
    PRIMARY KEY (batch_id, sample_id)
);

CREATE TABLE model_baseline_evaluations (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    version_id UUID NOT NULL REFERENCES model_baseline_versions(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL,
    candidates JSONB NOT NULL,
    results JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX model_baseline_evaluations_version_idx ON model_baseline_evaluations(version_id,created_at DESC);

-- Deep checks reuse the quality monitor job queue.
ALTER TABLE quality_monitor_jobs DROP CONSTRAINT quality_monitor_jobs_source_check;
ALTER TABLE quality_monitor_jobs ADD CONSTRAINT quality_monitor_jobs_source_check
    CHECK (source IN ('manual','scheduled','create','deep'));
ALTER TABLE quality_monitor_jobs
    ADD COLUMN claimed_model TEXT,
    ADD COLUMN options JSONB,
    ADD COLUMN target_samples INTEGER,
    ADD COLUMN completed_samples INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN valid_samples INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN apply_to_routing BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN verdict TEXT,
    ADD COLUMN evidence JSONB,
    ADD COLUMN stopped_early BOOLEAN NOT NULL DEFAULT false;

ALTER TABLE quality_monitors
    ADD COLUMN daily_sample_limit INTEGER NOT NULL DEFAULT 0 CHECK (daily_sample_limit BETWEEN 0 AND 100000),
    ADD COLUMN identity_verdict TEXT,
    ADD COLUMN identity_evidence JSONB,
    ADD COLUMN identity_checked_at TIMESTAMPTZ;

-- Users acknowledge once per channel that relays may spot and penalise probes.
ALTER TABLE channels
    ADD COLUMN authenticity_ack_at TIMESTAMPTZ,
    ADD COLUMN authenticity_ack_by UUID REFERENCES users(id) ON DELETE SET NULL;

CREATE TABLE authenticity_samples (
    id BIGSERIAL PRIMARY KEY,
    job_id UUID NOT NULL REFERENCES quality_monitor_jobs(id) ON DELETE CASCADE,
    monitor_id UUID NOT NULL REFERENCES quality_monitors(id) ON DELETE CASCADE,
    probe_kind TEXT NOT NULL CHECK (probe_kind IN ('fingerprint','token')),
    template_id UUID NOT NULL,
    n INTEGER,
    sample_values INTEGER[] NOT NULL DEFAULT '{}',
    input_tokens INTEGER,
    output_tokens INTEGER,
    reasoning_tokens INTEGER,
    status_code INTEGER,
    error_code TEXT,
    valid BOOLEAN NOT NULL,
    invalid_reason TEXT,
    latency_ms INTEGER,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX authenticity_samples_job_idx ON authenticity_samples(job_id,id);
CREATE INDEX authenticity_samples_daily_idx ON authenticity_samples(monitor_id,created_at);
