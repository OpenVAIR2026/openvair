-- Corrections to 0032-0034. Those migrations already ran in the test
-- environment, so they stay unchanged.

-- 0032 could copy a legacy prefix rule such as `demo-*` into a model monitor.
-- Such monitors can never probe. Carry their settings over to the channel's
-- first concrete model, then retire them.
INSERT INTO quality_monitors(channel_id,model,schedule_enabled,interval_seconds,timeout_ms,round_timeout_ms,
                             methods,min_score,failure_threshold,daily_run_limit,route_policy,next_run_at)
SELECT DISTINCT ON (w.channel_id) w.channel_id,candidate.model,w.schedule_enabled,w.interval_seconds,w.timeout_ms,
       w.round_timeout_ms,w.methods,w.min_score,w.failure_threshold,w.daily_run_limit,w.route_policy,
       CASE WHEN w.schedule_enabled THEN now() END
FROM quality_monitors w JOIN channels c ON c.id=w.channel_id
CROSS JOIN LATERAL (
    SELECT btrim(name) AS model FROM (
        SELECT mc.quality_model AS name,0 AS rank,0::bigint AS position
        FROM channel_monitor_configs mc WHERE mc.channel_id=c.id
        UNION ALL SELECT c.test_model,1,0
        UNION ALL SELECT rule,2,position FROM jsonb_array_elements_text(c.models) WITH ORDINALITY AS rules(rule,position)
        UNION ALL SELECT key,3,0 FROM jsonb_each_text(c.model_mapping)
        UNION ALL SELECT public_model,4,priority FROM model_routes
        WHERE channel_id=c.id AND account_id=c.account_id AND enabled
    ) names
    WHERE btrim(name) <> '' AND strpos(name,'*')=0 AND length(btrim(name)) <= 256
    ORDER BY rank,position,name LIMIT 1
) candidate
WHERE w.deleted_at IS NULL AND strpos(w.model,'*')>0
  AND NOT EXISTS (SELECT 1 FROM quality_monitors o
                  WHERE o.channel_id=w.channel_id AND o.deleted_at IS NULL AND strpos(o.model,'*')=0)
ORDER BY w.channel_id,w.created_at,w.id;

UPDATE quality_monitor_jobs j SET status='cancelled',finished_at=now(),error_code='monitor_deleted'
FROM quality_monitors q
WHERE j.monitor_id=q.id AND q.deleted_at IS NULL AND strpos(q.model,'*')>0 AND j.status IN ('queued','running');

UPDATE quality_monitors SET deleted_at=now(),schedule_enabled=false,next_run_at=NULL,valid_until=NULL,
    config_version=config_version+1,updated_at=now()
WHERE deleted_at IS NULL AND strpos(model,'*')>0;

ALTER TABLE quality_monitors ADD CONSTRAINT quality_monitor_concrete_model
    CHECK (deleted_at IS NOT NULL OR strpos(model,'*')=0);

-- A key allowing every model stores no rules. Versions before 0033 never write
-- model_mode, so a key they create or edit with rules would otherwise keep the
-- column default `all` during a deployment preflight or rollback.
UPDATE virtual_api_keys SET allowed_models='[]'::jsonb
WHERE model_mode='all' AND jsonb_array_length(allowed_models)>0;

CREATE FUNCTION restrict_virtual_key_model_mode() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.model_mode='all' AND jsonb_array_length(NEW.allowed_models)>0 THEN
        NEW.model_mode := 'allowlist';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER virtual_keys_model_mode BEFORE INSERT OR UPDATE ON virtual_api_keys
    FOR EACH ROW EXECUTE FUNCTION restrict_virtual_key_model_mode();

-- Before 0034 the daily cost limit skipped unpriced models instead of blocking
-- them. Keep that behaviour for existing keys; `block` is an explicit choice.
UPDATE virtual_api_keys SET unknown_cost_policy='allow' WHERE unknown_cost_policy='block';
