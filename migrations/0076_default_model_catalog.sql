-- Current flagship models from the major international and mainland vendors.
-- IDs follow OpenRouter's native names (Anthropic versions use dashes) so the
-- price sync can fill them in; channels can map them to a vendor's own IDs.
-- Rows an administrator already edited keep their vendor and protocols.
INSERT INTO model_catalog(pattern, label, provider, protocols) VALUES
    ('gpt-6.1-sol', 'GPT-6.1 Sol', 'OpenAI', ARRAY['openai']),
    ('gpt-6.1-terra', 'GPT-6.1 Terra', 'OpenAI', ARRAY['openai']),
    ('gpt-6-sol', 'GPT-6 Sol', 'OpenAI', ARRAY['openai']),
    ('gpt-6-luna', 'GPT-6 Luna', 'OpenAI', ARRAY['openai']),
    ('gpt-5.5', 'GPT-5.5', 'OpenAI', ARRAY['openai']),
    ('gpt-5.4', 'GPT-5.4', 'OpenAI', ARRAY['openai']),
    ('gpt-5.4-mini', 'GPT-5.4 Mini', 'OpenAI', ARRAY['openai']),
    ('claude-fable-5-1', 'Claude Fable 5.1', 'Anthropic', ARRAY['anthropic','openai']),
    ('claude-opus-5-5', 'Claude Opus 5.5', 'Anthropic', ARRAY['anthropic','openai']),
    ('claude-sonnet-5-5', 'Claude Sonnet 5.5', 'Anthropic', ARRAY['anthropic','openai']),
    ('claude-haiku-5-5', 'Claude Haiku 5.5', 'Anthropic', ARRAY['anthropic','openai']),
    ('claude-opus-5', 'Claude Opus 5', 'Anthropic', ARRAY['anthropic','openai']),
    ('claude-sonnet-5', 'Claude Sonnet 5', 'Anthropic', ARRAY['anthropic','openai']),
    ('claude-haiku-4-5', 'Claude Haiku 4.5', 'Anthropic', ARRAY['anthropic','openai']),
    ('gemini-4-argon', 'Gemini 4 Argon', 'Google', ARRAY['gemini','openai']),
    ('deepseek-v4.1-flash', 'DeepSeek V4.1 Flash', 'DeepSeek', ARRAY['openai','anthropic']),
    ('deepseek-v4-pro', 'DeepSeek V4 Pro', 'DeepSeek', ARRAY['openai','anthropic']),
    ('deepseek-v4-flash', 'DeepSeek V4 Flash', 'DeepSeek', ARRAY['openai','anthropic']),
    ('qwen3.8-max-prime', 'Qwen3.8 Max Prime', '通义千问（阿里云百炼）', ARRAY['openai','anthropic']),
    ('qwen3.8-flash', 'Qwen3.8 Flash', '通义千问（阿里云百炼）', ARRAY['openai','anthropic']),
    ('qwen3.7-max', 'Qwen3.7 Max', '通义千问（阿里云百炼）', ARRAY['openai','anthropic']),
    ('qwen3.7-plus', 'Qwen3.7 Plus', '通义千问（阿里云百炼）', ARRAY['openai','anthropic']),
    ('kimi-k3.1-pro', 'Kimi K3.1 Pro', 'Kimi（月之暗面）', ARRAY['openai','anthropic']),
    ('kimi-k3', 'Kimi K3', 'Kimi（月之暗面）', ARRAY['openai','anthropic']),
    ('kimi-k2.7-code', 'Kimi K2.7 Code', 'Kimi（月之暗面）', ARRAY['openai','anthropic']),
    ('glm-5.3-prime', 'GLM-5.3 Prime', '智谱 GLM', ARRAY['openai','anthropic']),
    ('glm-5.3', 'GLM-5.3', '智谱 GLM', ARRAY['openai','anthropic']),
    ('glm-5.3-flash', 'GLM-5.3 Flash', '智谱 GLM', ARRAY['openai','anthropic']),
    ('minimax-m3', 'MiniMax M3', 'MiniMax', ARRAY['openai','anthropic']),
    ('minimax-m3.1-flash', 'MiniMax M3.1 Flash', 'MiniMax', ARRAY['openai','anthropic']),
    ('seed-2.0-code', '豆包 Seed 2.0 Code', '豆包（火山方舟）', ARRAY['openai','anthropic']),
    ('seed-2-1-turbo', '豆包 Seed 2.1 Turbo', '豆包（火山方舟）', ARRAY['openai','anthropic'])
ON CONFLICT (pattern) DO UPDATE
SET provider=EXCLUDED.provider, protocols=EXCLUDED.protocols, updated_at=now()
WHERE model_catalog.provider='';

-- Earlier seeds were added without a vendor and offered on every protocol.
UPDATE model_catalog SET
    provider=CASE
        WHEN pattern ~ '^(gpt-|o[0-9])' THEN 'OpenAI'
        WHEN pattern LIKE 'claude-%' THEN 'Anthropic'
        WHEN pattern LIKE 'gemini-%' THEN 'Google'
        WHEN pattern LIKE 'deepseek-%' THEN 'DeepSeek'
        WHEN pattern LIKE 'llama-%' THEN 'Meta'
        ELSE 'Mistral' END,
    protocols=CASE
        WHEN pattern LIKE 'claude-%' THEN ARRAY['anthropic','openai']
        WHEN pattern LIKE 'gemini-%' THEN ARRAY['gemini','openai']
        WHEN pattern LIKE 'deepseek-%' THEN ARRAY['openai','anthropic']
        ELSE ARRAY['openai'] END,
    updated_at=now()
WHERE strpos(pattern,'*')=0 AND provider='' AND protocols=ARRAY['openai','anthropic','gemini']
  AND pattern ~ '^(gpt-|o[0-9]|claude-|gemini-|deepseek-|llama-|mistral-|codestral-)';
