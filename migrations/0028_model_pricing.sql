-- Model pricing catalog, stored in system_settings as JSON under key 'model_pricing'.
-- Each entry: { pattern, label, input_per_1m, output_per_1m, cached_read_per_1m, synced_from }
-- No schema change needed; seeding a sensible default set here as a no-op JSON upsert.

INSERT INTO system_settings (key, value_ciphertext)
VALUES ('model_pricing', '[
  {"pattern":"gpt-4o-mini","label":"GPT-4o mini","input_per_1m":0.15,"output_per_1m":0.60,"cached_read_per_1m":0.075,"synced_from":null},
  {"pattern":"gpt-4o","label":"GPT-4o","input_per_1m":2.50,"output_per_1m":10.00,"cached_read_per_1m":1.25,"synced_from":null},
  {"pattern":"gpt-4.1-mini","label":"GPT-4.1 mini","input_per_1m":0.40,"output_per_1m":1.60,"cached_read_per_1m":0.10,"synced_from":null},
  {"pattern":"gpt-4.1","label":"GPT-4.1","input_per_1m":2.00,"output_per_1m":8.00,"cached_read_per_1m":0.50,"synced_from":null},
  {"pattern":"o1-mini","label":"o1 mini","input_per_1m":1.10,"output_per_1m":4.40,"cached_read_per_1m":0.55,"synced_from":null},
  {"pattern":"o1","label":"o1","input_per_1m":15.00,"output_per_1m":60.00,"cached_read_per_1m":7.50,"synced_from":null},
  {"pattern":"o3-mini","label":"o3 mini","input_per_1m":1.10,"output_per_1m":4.40,"cached_read_per_1m":0.55,"synced_from":null},
  {"pattern":"o3","label":"o3","input_per_1m":10.00,"output_per_1m":40.00,"cached_read_per_1m":2.50,"synced_from":null},
  {"pattern":"claude-opus-4","label":"Claude Opus 4","input_per_1m":15.00,"output_per_1m":75.00,"cached_read_per_1m":1.50,"synced_from":null},
  {"pattern":"claude-sonnet-4","label":"Claude Sonnet 4","input_per_1m":3.00,"output_per_1m":15.00,"cached_read_per_1m":0.30,"synced_from":null},
  {"pattern":"claude-haiku-4","label":"Claude Haiku 4","input_per_1m":0.80,"output_per_1m":4.00,"cached_read_per_1m":0.08,"synced_from":null},
  {"pattern":"claude-opus-5","label":"Claude Opus 5","input_per_1m":15.00,"output_per_1m":75.00,"cached_read_per_1m":1.50,"synced_from":null},
  {"pattern":"claude-sonnet-5","label":"Claude Sonnet 5","input_per_1m":3.00,"output_per_1m":15.00,"cached_read_per_1m":0.30,"synced_from":null},
  {"pattern":"gemini-2.0-flash","label":"Gemini 2.0 Flash","input_per_1m":0.075,"output_per_1m":0.30,"cached_read_per_1m":0.01875,"synced_from":null},
  {"pattern":"gemini-2.5-flash","label":"Gemini 2.5 Flash","input_per_1m":0.15,"output_per_1m":0.60,"cached_read_per_1m":0.0375,"synced_from":null},
  {"pattern":"gemini-2.5-pro","label":"Gemini 2.5 Pro","input_per_1m":1.25,"output_per_1m":5.00,"cached_read_per_1m":0.3125,"synced_from":null}
]')
ON CONFLICT (key) DO NOTHING;
