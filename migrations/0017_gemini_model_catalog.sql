-- Model IDs verified against https://ai.google.dev/gemini-api/docs/models.
-- Catalog entries are permission suggestions; channels determine availability.
INSERT INTO model_catalog(pattern, label) VALUES
    ('gemini-2.5-*', 'Gemini 2.5 系列'),
    ('gemini-3.8-flash', 'Gemini 3.8 Flash'),
    ('gemini-3.7-flash', 'Gemini 3.7 Flash'),
    ('gemini-3.6-flash', 'Gemini 3.6 Flash'),
    ('gemini-3.5-flash', 'Gemini 3.5 Flash'),
    ('gemini-3.5-flash-lite', 'Gemini 3.5 Flash-Lite'),
    ('gemini-3.1-flash-lite', 'Gemini 3.1 Flash-Lite'),
    ('gemini-3.1-pro-preview', 'Gemini 3.1 Pro（预览）'),
    ('gemini-3-flash-preview', 'Gemini 3 Flash（预览）'),
    ('gemini-2.5-pro', 'Gemini 2.5 Pro'),
    ('gemini-2.5-flash', 'Gemini 2.5 Flash'),
    ('gemini-2.5-flash-lite', 'Gemini 2.5 Flash-Lite')
ON CONFLICT (pattern) DO NOTHING;
