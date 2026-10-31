-- Metering warnings already emailed, shared by passive metering and service
-- checks so one problem sends one email and survives hours without samples.
CREATE TABLE metering_alerts (
    channel_id UUID NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    upstream_model TEXT NOT NULL,
    kind TEXT NOT NULL,
    source TEXT NOT NULL,
    alerted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (channel_id, upstream_model, kind)
);

-- Warnings emailed under the old rules stay alerted.
INSERT INTO metering_alerts (channel_id, upstream_model, kind, source, alerted_at)
SELECT s.channel_id, s.upstream_model,
       CASE WHEN a.kind LIKE 'inflated%' THEN 'inflated' ELSE a.kind END,
       'passive', s.computed_at
FROM channel_passive_stats s, jsonb_array_elements_text(s.alerted) AS a(kind)
ON CONFLICT DO NOTHING;
