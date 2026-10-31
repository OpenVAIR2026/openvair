-- Platform pricing discovery scans recent traffic across accounts.
CREATE INDEX usage_created_idx ON usage_events(created_at DESC);
