-- Quality monitoring is a model-scoped resource. Legacy configuration and
-- channel-level history remain available; aggregate scores are not migrated.
CREATE TABLE quality_monitors (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    channel_id UUID NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    model TEXT NOT NULL CHECK (length(btrim(model)) BETWEEN 1 AND 256),
    schedule_enabled BOOLEAN NOT NULL DEFAULT false,
    interval_seconds INTEGER NOT NULL DEFAULT 3600 CHECK (interval_seconds BETWEEN 900 AND 86400),
    timeout_ms INTEGER NOT NULL DEFAULT 60000 CHECK (timeout_ms BETWEEN 10000 AND 180000),
    round_timeout_ms INTEGER NOT NULL DEFAULT 240000 CHECK (round_timeout_ms BETWEEN 10000 AND 720000),
    methods JSONB NOT NULL DEFAULT '["arithmetic","json","instruction","context"]',
    min_score DOUBLE PRECISION NOT NULL DEFAULT 75 CHECK (min_score BETWEEN 0 AND 100),
    failure_threshold INTEGER NOT NULL DEFAULT 2 CHECK (failure_threshold BETWEEN 1 AND 5),
    daily_run_limit INTEGER NOT NULL DEFAULT 0 CHECK (daily_run_limit BETWEEN 0 AND 10000),
    route_policy TEXT NOT NULL DEFAULT 'observe' CHECK (route_policy IN ('observe','deprioritize','exclude')),
    config_version BIGINT NOT NULL DEFAULT 1,
    status TEXT NOT NULL DEFAULT 'unknown' CHECK (status IN ('unknown','healthy','degraded')),
    last_observation TEXT NOT NULL DEFAULT 'unknown',
    score DOUBLE PRECISION,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    checked_at TIMESTAMPTZ,
    valid_until TIMESTAMPTZ,
    next_run_at TIMESTAMPTZ,
    schedule_error TEXT,
    deleted_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX quality_monitors_active_model_idx ON quality_monitors(channel_id,model) WHERE deleted_at IS NULL;
CREATE INDEX quality_monitors_due_idx ON quality_monitors(next_run_at) WHERE schedule_enabled AND deleted_at IS NULL;

INSERT INTO quality_monitors(channel_id,model,schedule_enabled,interval_seconds,timeout_ms,round_timeout_ms,
                             methods,min_score,failure_threshold,route_policy,next_run_at)
SELECT DISTINCT c.id,btrim(selected.model),mc.quality_enabled,mc.quality_interval_seconds,mc.quality_timeout_ms,
       mc.quality_timeout_ms * 4,mc.quality_methods,mc.quality_min_score,mc.quality_failure_threshold,
       CASE WHEN mc.quality_enabled THEN 'exclude' ELSE 'observe' END,
       CASE WHEN mc.quality_enabled THEN now() END
FROM channels c JOIN channel_monitor_configs mc ON mc.channel_id=c.id
LEFT JOIN LATERAL (
    SELECT public_model FROM model_routes WHERE channel_id=c.id AND account_id=c.account_id AND enabled
    ORDER BY priority,id LIMIT 1
) route ON true
CROSS JOIN LATERAL jsonb_array_elements_text(
    CASE WHEN EXISTS (SELECT 1 FROM jsonb_array_elements_text(mc.quality_models) m WHERE btrim(m) <> '')
         THEN mc.quality_models
         ELSE jsonb_build_array(COALESCE(NULLIF(btrim(mc.quality_model),''),NULLIF(btrim(c.test_model),''),
              NULLIF(btrim(c.models->>0),''),(SELECT key FROM jsonb_each_text(c.model_mapping) ORDER BY key LIMIT 1),route.public_model)) END
) selected(model)
WHERE (mc.quality_enabled OR jsonb_array_length(mc.quality_models)>0 OR NULLIF(btrim(mc.quality_model),'') IS NOT NULL
       OR EXISTS (SELECT 1 FROM channel_monitor_runs r WHERE r.channel_id=c.id AND r.kind='quality'))
  AND selected.model IS NOT NULL AND btrim(selected.model)<>'';

CREATE TABLE quality_monitor_jobs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    monitor_id UUID NOT NULL REFERENCES quality_monitors(id) ON DELETE CASCADE,
    config_version BIGINT NOT NULL,
    source TEXT NOT NULL CHECK (source IN ('manual','scheduled','create')),
    status TEXT NOT NULL DEFAULT 'queued' CHECK (status IN ('queued','running','completed','failed','cancelled')),
    error_code TEXT,
    lease_until TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ
);
CREATE UNIQUE INDEX quality_monitor_jobs_active_idx ON quality_monitor_jobs(monitor_id) WHERE status IN ('queued','running');
CREATE INDEX quality_monitor_jobs_queue_idx ON quality_monitor_jobs(created_at) WHERE status='queued';
CREATE INDEX quality_monitor_jobs_daily_idx ON quality_monitor_jobs(monitor_id,created_at);

ALTER TABLE channel_monitor_runs ADD COLUMN monitor_id UUID REFERENCES quality_monitors(id) ON DELETE SET NULL;
CREATE INDEX channel_monitor_runs_monitor_idx ON channel_monitor_runs(monitor_id,created_at DESC,id DESC) WHERE monitor_id IS NOT NULL;

-- Any channel change invalidates in-flight results and model routing decisions.
-- In particular, re-enabling a channel queues a fresh scheduled observation.
CREATE FUNCTION invalidate_channel_quality_monitors() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.enabled IS DISTINCT FROM NEW.enabled OR OLD.base_url IS DISTINCT FROM NEW.base_url
       OR OLD.model_mapping IS DISTINCT FROM NEW.model_mapping OR OLD.models IS DISTINCT FROM NEW.models
       OR OLD.model_mode IS DISTINCT FROM NEW.model_mode OR OLD.protocol IS DISTINCT FROM NEW.protocol THEN
        UPDATE quality_monitors SET config_version=config_version+1,status='unknown',last_observation='unknown',
            score=NULL,consecutive_failures=0,last_error=NULL,checked_at=NULL,valid_until=NULL,
            next_run_at=CASE WHEN schedule_enabled THEN now() END,updated_at=now()
        WHERE channel_id=NEW.id AND deleted_at IS NULL;
        UPDATE quality_monitor_jobs j SET status='cancelled',finished_at=now(),error_code='channel_changed'
        FROM quality_monitors q WHERE j.monitor_id=q.id AND q.channel_id=NEW.id AND j.status IN ('queued','running');
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER channels_quality_changed AFTER UPDATE ON channels FOR EACH ROW EXECUTE FUNCTION invalidate_channel_quality_monitors();

-- Legacy route APIs can change an alias without editing channels.model_mapping.
-- Such a change must not reuse the previous upstream model's quality verdict.
CREATE FUNCTION invalidate_route_quality_monitors() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'UPDATE' AND OLD.channel_id IS NOT DISTINCT FROM NEW.channel_id
       AND OLD.public_model IS NOT DISTINCT FROM NEW.public_model
       AND OLD.upstream_model IS NOT DISTINCT FROM NEW.upstream_model
       AND OLD.protocol IS NOT DISTINCT FROM NEW.protocol
       AND OLD.priority IS NOT DISTINCT FROM NEW.priority
       AND OLD.enabled IS NOT DISTINCT FROM NEW.enabled THEN
        RETURN NULL;
    END IF;
    IF TG_OP <> 'INSERT' THEN
        UPDATE quality_monitors SET config_version=config_version+1,status='unknown',last_observation='unknown',
            score=NULL,consecutive_failures=0,last_error=NULL,checked_at=NULL,valid_until=NULL,
            next_run_at=CASE WHEN schedule_enabled THEN now() END,updated_at=now()
        WHERE channel_id=OLD.channel_id AND model=OLD.public_model AND deleted_at IS NULL;
        UPDATE quality_monitor_jobs j SET status='cancelled',finished_at=now(),error_code='route_changed'
        FROM quality_monitors q WHERE j.monitor_id=q.id AND q.channel_id=OLD.channel_id AND q.model=OLD.public_model
            AND j.config_version<>q.config_version AND j.status IN ('queued','running');
    END IF;
    IF TG_OP <> 'DELETE' THEN
        UPDATE quality_monitors SET config_version=config_version+1,status='unknown',last_observation='unknown',
            score=NULL,consecutive_failures=0,last_error=NULL,checked_at=NULL,valid_until=NULL,
            next_run_at=CASE WHEN schedule_enabled THEN now() END,updated_at=now()
        WHERE channel_id=NEW.channel_id AND model=NEW.public_model AND deleted_at IS NULL;
        UPDATE quality_monitor_jobs j SET status='cancelled',finished_at=now(),error_code='route_changed'
        FROM quality_monitors q WHERE j.monitor_id=q.id AND q.channel_id=NEW.channel_id AND q.model=NEW.public_model
            AND j.config_version<>q.config_version AND j.status IN ('queued','running');
    END IF;
    RETURN NULL;
END $$;
CREATE TRIGGER model_routes_quality_changed AFTER INSERT OR UPDATE OR DELETE ON model_routes FOR EACH ROW EXECUTE FUNCTION invalidate_route_quality_monitors();
