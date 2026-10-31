ALTER TABLE channel_monitor_configs
    ADD COLUMN IF NOT EXISTS quality_models JSONB NOT NULL DEFAULT '[]'::jsonb,
    ADD COLUMN IF NOT EXISTS quality_methods JSONB NOT NULL DEFAULT '["arithmetic","json","instruction","context"]'::jsonb;
