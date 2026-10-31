-- The closest model used to be the highest total likelihood, which could
-- name a model other than the one the verdict weighed. It is now the
-- candidate with the most negative z, or the claim when no candidate is
-- favoured; stored reports and monitor summaries are recomputed the same way.
UPDATE quality_monitor_jobs j
SET evidence = jsonb_set(j.evidence, '{best_model}', to_jsonb(
    CASE WHEN c.z < 0 THEN c.model ELSE j.evidence->>'claimed' END))
FROM (
    SELECT DISTINCT ON (jj.id) jj.id, x->>'model' AS model, (x->>'z')::float8 AS z
    FROM quality_monitor_jobs jj, jsonb_array_elements(jj.evidence->'comparisons') x
    WHERE jsonb_typeof(jj.evidence->'comparisons') = 'array'
    ORDER BY jj.id, (x->>'z')::float8
) c
WHERE c.id = j.id AND j.evidence->>'claimed' IS NOT NULL;

UPDATE quality_monitors q
SET identity_evidence = jsonb_set(q.identity_evidence, '{best_model}', j.evidence->'best_model')
FROM quality_monitor_jobs j
WHERE q.identity_evidence ? 'job_id' AND j.id = (q.identity_evidence->>'job_id')::uuid
  AND j.evidence ? 'best_model';

UPDATE quality_monitors q
SET window_evidence = jsonb_set(q.window_evidence, '{best_model}', COALESCE((
    SELECT evidence->'best_model' FROM quality_monitor_jobs
    WHERE monitor_id = q.id AND source = 'scheduled' AND status = 'completed' AND evidence ? 'best_model'
    ORDER BY finished_at DESC NULLS LAST, created_at DESC LIMIT 1
), q.window_evidence->'best_model', 'null'::jsonb))
WHERE q.window_evidence IS NOT NULL;
