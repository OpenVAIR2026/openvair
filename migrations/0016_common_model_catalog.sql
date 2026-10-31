INSERT INTO model_catalog(pattern, label) VALUES
    ('gpt-5*', 'GPT-5 系列'),
    ('o3*', 'OpenAI o3 系列'),
    ('o4*', 'OpenAI o4 系列'),
    ('claude-5*', 'Claude 5 系列'),
    ('gemini-3*', 'Gemini 3 系列'),
    ('deepseek-v4-*', 'DeepSeek V4 系列'),
    ('deepseek-*', 'DeepSeek 系列'),
    ('qwen3-*', 'Qwen3 系列'),
    ('qwen-*', 'Qwen 系列'),
    ('llama-4-*', 'Llama 4 系列'),
    ('llama-*', 'Llama 系列'),
    ('mistral-*', 'Mistral 系列'),
    ('glm-*', 'GLM 系列'),
    ('kimi-*', 'Kimi 系列')
ON CONFLICT (pattern) DO NOTHING;
