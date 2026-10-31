-- Curated model IDs from the providers' current public model catalogs.
-- These entries are suggestions for API Key permissions; channel routes still
-- decide which models can actually receive traffic.
INSERT INTO model_catalog(pattern, label) VALUES
    ('gpt-5.6', 'GPT-5.6'),
    ('gpt-5.6-sol', 'GPT-5.6 Sol'),
    ('gpt-5.6-terra', 'GPT-5.6 Terra'),
    ('gpt-5.6-luna', 'GPT-5.6 Luna'),
    ('gpt-4.1', 'GPT-4.1'),
    ('gpt-4.1-mini', 'GPT-4.1 mini'),
    ('o3', 'OpenAI o3'),
    ('o4-mini', 'OpenAI o4-mini'),
    ('claude-opus-5-5', 'Claude Opus 5.5'),
    ('claude-sonnet-5-5', 'Claude Sonnet 5.5'),
    ('claude-haiku-4-5', 'Claude Haiku 4.5'),
    ('deepseek-v4-pro', 'DeepSeek V4 Pro'),
    ('deepseek-v4-flash', 'DeepSeek V4 Flash'),
    ('llama-4-maverick', 'Llama 4 Maverick'),
    ('llama-4-scout', 'Llama 4 Scout'),
    ('mistral-large-latest', 'Mistral Large'),
    ('mistral-small-latest', 'Mistral Small'),
    ('codestral-latest', 'Codestral')
ON CONFLICT (pattern) DO NOTHING;
