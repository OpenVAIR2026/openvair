-- Public model verification: anyone with a verified email can check one
-- endpoint against the baselines without an account. The API key and full
-- base URL live only in Redis while the check runs; rows keep the host.
CREATE TABLE public_check_verifications (
    email TEXT PRIMARY KEY,
    id UUID NOT NULL UNIQUE,
    code_hash TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts BETWEEN 0 AND 5),
    expires_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX public_check_verifications_expiry_idx ON public_check_verifications(expires_at);

CREATE TABLE public_checks (
    id UUID PRIMARY KEY,
    token TEXT NOT NULL UNIQUE,
    manage_token_hash TEXT NOT NULL,
    email_hash TEXT NOT NULL,
    ip_hash TEXT,
    protocol TEXT NOT NULL CHECK (protocol IN ('openai', 'anthropic', 'gemini')),
    host TEXT NOT NULL,
    show_host BOOLEAN NOT NULL DEFAULT false,
    claimed_model TEXT NOT NULL,
    upstream_model TEXT NOT NULL,
    baseline_version_id UUID,
    target_samples INTEGER NOT NULL CHECK (target_samples > 0),
    completed_samples INTEGER NOT NULL DEFAULT 0,
    valid_samples INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'queued'
        CHECK (status IN ('queued', 'running', 'done', 'failed', 'deleted')),
    verdict TEXT,
    -- Public report data only: never the key, URL path, prompts or answers.
    progress JSONB,
    evidence JSONB,
    error_code TEXT,
    stopped_early BOOLEAN NOT NULL DEFAULT false,
    input_tokens BIGINT NOT NULL DEFAULT 0,
    output_tokens BIGINT NOT NULL DEFAULT 0,
    cost_usd DOUBLE PRECISION,
    lease_until TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    expires_at TIMESTAMPTZ NOT NULL DEFAULT now() + interval '180 days'
);
CREATE INDEX public_checks_queue_idx ON public_checks(created_at) WHERE status = 'queued';
CREATE INDEX public_checks_running_idx ON public_checks(lease_until) WHERE status = 'running';

CREATE TABLE public_check_samples (
    id BIGSERIAL PRIMARY KEY,
    check_id UUID NOT NULL REFERENCES public_checks(id) ON DELETE CASCADE,
    probe_kind TEXT NOT NULL CHECK (probe_kind IN ('fingerprint', 'token')),
    template_id UUID NOT NULL,
    n INTEGER,
    sample_values INTEGER[] NOT NULL DEFAULT '{}',
    input_tokens INTEGER,
    output_tokens INTEGER,
    status_code INTEGER,
    error_code TEXT,
    valid BOOLEAN NOT NULL,
    invalid_reason TEXT,
    ttfb_ms INTEGER,
    latency_ms INTEGER NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX public_check_samples_check_idx ON public_check_samples(check_id);
