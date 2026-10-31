-- The global catalog is the source of concrete model identities and exact prices.
-- Prefix entries remain selection templates; they can never carry a price.
ALTER TABLE model_catalog
    ADD COLUMN provider TEXT NOT NULL DEFAULT '',
    ADD COLUMN protocols TEXT[] NOT NULL DEFAULT ARRAY['openai','anthropic','gemini'],
    ADD COLUMN input_per_1m DOUBLE PRECISION,
    ADD COLUMN output_per_1m DOUBLE PRECISION,
    ADD COLUMN cached_read_per_1m DOUBLE PRECISION,
    ADD COLUMN price_source TEXT,
    ADD CONSTRAINT model_catalog_price CHECK (
        (input_per_1m IS NULL AND output_per_1m IS NULL AND cached_read_per_1m IS NULL)
        OR (strpos(pattern, '*') = 0
            AND input_per_1m >= 0 AND input_per_1m < 'Infinity'::float8
            AND output_per_1m >= 0 AND output_per_1m < 'Infinity'::float8
            AND cached_read_per_1m >= 0 AND cached_read_per_1m < 'Infinity'::float8
            AND input_per_1m IS NOT NULL AND output_per_1m IS NOT NULL AND cached_read_per_1m IS NOT NULL)
    );

-- Only an exact existing concrete identity is eligible for automatic migration.
-- Keep the original setting as a recoverable archive for unresolved legacy prices.
UPDATE model_catalog m
SET input_per_1m=(p->>'input_per_1m')::float8,
    output_per_1m=(p->>'output_per_1m')::float8,
    cached_read_per_1m=(p->>'cached_read_per_1m')::float8,
    price_source=p->>'synced_from'
FROM system_settings s, LATERAL jsonb_array_elements(s.value_ciphertext::jsonb) p
WHERE s.key='model_pricing' AND m.pattern=p->>'pattern' AND strpos(m.pattern,'*')=0
  AND jsonb_typeof(p->'input_per_1m')='number'
  AND jsonb_typeof(p->'output_per_1m')='number'
  AND jsonb_typeof(p->'cached_read_per_1m')='number'
  AND (p->>'input_per_1m')::float8 >= 0
  AND (p->>'output_per_1m')::float8 >= 0
  AND (p->>'cached_read_per_1m')::float8 >= 0;

ALTER TABLE channels
    ADD COLUMN discovered_models JSONB NOT NULL DEFAULT '[]'::jsonb,
    ADD COLUMN models_synced_at TIMESTAMPTZ;

ALTER TABLE virtual_api_keys ADD COLUMN model_mode TEXT NOT NULL DEFAULT 'all';
UPDATE virtual_api_keys SET model_mode='allowlist' WHERE jsonb_array_length(allowed_models)>0;
ALTER TABLE virtual_api_keys ADD CONSTRAINT virtual_key_model_mode CHECK (model_mode IN ('all','allowlist'));

CREATE TABLE account_models (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    model TEXT NOT NULL CHECK (length(btrim(model)) BETWEEN 1 AND 256 AND strpos(model,'*')=0),
    label TEXT NOT NULL,
    protocol TEXT NOT NULL CHECK (protocol IN ('openai','anthropic','gemini')),
    status TEXT NOT NULL DEFAULT 'registered' CHECK (status IN ('registered','pending')),
    source TEXT NOT NULL DEFAULT 'manual',
    last_seen_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(account_id,model,protocol)
);

-- Preserve account-specific concrete names, without publishing them globally.
INSERT INTO account_models(account_id,model,label,protocol,source)
SELECT DISTINCT c.account_id, names.model, names.model, c.protocol, 'channel'
FROM channels c CROSS JOIN LATERAL (
    SELECT jsonb_array_elements_text(c.models) AS model
    UNION SELECT jsonb_object_keys(c.model_mapping)
    UNION SELECT r.public_model FROM model_routes r
      WHERE r.channel_id=c.id AND r.account_id=c.account_id AND r.enabled AND r.protocol=c.protocol
) names
WHERE length(btrim(names.model)) BETWEEN 1 AND 256 AND strpos(names.model,'*')=0
  AND NOT EXISTS (SELECT 1 FROM model_catalog m WHERE m.pattern=names.model AND c.protocol=ANY(m.protocols))
ON CONFLICT DO NOTHING;

ALTER TABLE usage_events
    ADD COLUMN upstream_model TEXT,
    ADD COLUMN price_snapshot JSONB,
    ADD COLUMN cost_usd DOUBLE PRECISION,
    ADD COLUMN pricing_status TEXT NOT NULL DEFAULT 'legacy'
        CHECK (pricing_status IN ('legacy','priced','unpriced','pending_usage'));
CREATE INDEX usage_unpriced_upstream_idx ON usage_events(created_at DESC,upstream_model)
    WHERE upstream_model IS NOT NULL;
