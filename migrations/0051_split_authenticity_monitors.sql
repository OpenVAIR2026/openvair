-- Service checks and authenticity checks become separate monitors. Each
-- channel and model may have one of each; only service monitors steer routing.
ALTER TABLE quality_monitors
    ADD COLUMN kind TEXT NOT NULL DEFAULT 'service' CHECK (kind IN ('service','authenticity'));

DROP INDEX quality_monitors_active_model_idx;
CREATE UNIQUE INDEX quality_monitors_active_model_idx
    ON quality_monitors(channel_id,model,kind) WHERE deleted_at IS NULL;

-- Deep checks in flight belong to the combined monitor being split.
UPDATE quality_monitor_jobs SET status='cancelled', finished_at=now(), error_code='monitor_split'
WHERE source='deep' AND status IN ('queued','running');

-- Monitors with authenticity history get their own authenticity monitor.
INSERT INTO quality_monitors(channel_id,model,kind,identity_verdict,identity_evidence,identity_checked_at,
                             daily_sample_limit,created_at)
SELECT q.channel_id,q.model,'authenticity',q.identity_verdict,q.identity_evidence,q.identity_checked_at,
       q.daily_sample_limit,q.created_at
FROM quality_monitors q
WHERE q.deleted_at IS NULL AND q.kind='service'
  AND (q.identity_checked_at IS NOT NULL
       OR EXISTS (SELECT 1 FROM quality_monitor_jobs j WHERE j.monitor_id=q.id AND j.source='deep'));

UPDATE quality_monitor_jobs j SET monitor_id=a.id
FROM quality_monitors s
JOIN quality_monitors a ON a.channel_id=s.channel_id AND a.model=s.model
    AND a.kind='authenticity' AND a.deleted_at IS NULL
WHERE j.monitor_id=s.id AND s.kind='service' AND s.deleted_at IS NULL AND j.source='deep';

UPDATE authenticity_samples x SET monitor_id=a.id
FROM quality_monitors s
JOIN quality_monitors a ON a.channel_id=s.channel_id AND a.model=s.model
    AND a.kind='authenticity' AND a.deleted_at IS NULL
WHERE x.monitor_id=s.id AND s.kind='service' AND s.deleted_at IS NULL;

UPDATE quality_monitors SET identity_verdict=NULL, identity_evidence=NULL, identity_checked_at=NULL
WHERE kind='service';
