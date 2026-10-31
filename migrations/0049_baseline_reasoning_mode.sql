-- Some models (e.g. Claude Sonnet 5.5 on OpenRouter) reject requests that turn
-- reasoning off. Such baselines are collected and checked with the model's
-- default reasoning ('default': no reasoning parameter is sent) instead.
ALTER TABLE model_baseline_targets
    ADD COLUMN reasoning TEXT NOT NULL DEFAULT 'off' CHECK (reasoning IN ('off','default'));
