-- Replacing a relay's source key changes the upstream behind its monitors.
-- Use the existing channel-change invalidation so settings/history survive,
-- old routing verdicts expire, and queued or in-flight jobs cannot apply them.
CREATE OR REPLACE FUNCTION invalidate_channel_quality_monitors() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.enabled IS DISTINCT FROM NEW.enabled OR OLD.base_url IS DISTINCT FROM NEW.base_url
       OR OLD.model_mapping IS DISTINCT FROM NEW.model_mapping OR OLD.models IS DISTINCT FROM NEW.models
       OR OLD.model_mode IS DISTINCT FROM NEW.model_mode OR OLD.protocol IS DISTINCT FROM NEW.protocol
       OR OLD.source_key_id IS DISTINCT FROM NEW.source_key_id THEN
        UPDATE quality_monitors SET config_version=config_version+1,status='unknown',last_observation='unknown',
            score=NULL,consecutive_failures=0,last_error=NULL,checked_at=NULL,valid_until=NULL,schedule_error=NULL,
            next_run_at=CASE WHEN schedule_enabled THEN now() END,updated_at=now()
        WHERE channel_id=NEW.id AND deleted_at IS NULL;
        UPDATE quality_monitor_jobs j SET status='cancelled',finished_at=now(),error_code='channel_changed'
        FROM quality_monitors q WHERE j.monitor_id=q.id AND q.channel_id=NEW.id AND j.status IN ('queued','running');
    END IF;
    RETURN NEW;
END $$;
