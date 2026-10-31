-- Quality monitoring suite v3: the four fixed basic questions become
-- randomized service checks (integrity and replay, metering, tool calling).
-- Old method names are no longer valid, so every monitor moves to the new
-- set and starts from an unknown result; history keeps its suite version.
UPDATE quality_monitors
SET methods='["integrity","metering","tools"]'::jsonb,
    config_version=config_version+1,
    status='unknown', last_observation='unknown', score=NULL, consecutive_failures=0,
    last_error=NULL, checked_at=NULL, valid_until=NULL, schedule_error=NULL,
    next_run_at=CASE WHEN schedule_enabled THEN now() END,
    updated_at=now()
WHERE deleted_at IS NULL;

UPDATE quality_monitor_jobs SET status='cancelled', finished_at=now(), error_code='suite_upgraded'
WHERE status IN ('queued','running') AND source<>'deep';

ALTER TABLE quality_monitors ALTER COLUMN methods SET DEFAULT '["integrity","metering","tools"]'::jsonb;
