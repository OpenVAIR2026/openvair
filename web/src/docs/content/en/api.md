Requests and responses use each vendor's official format: swap the address for `{{origin}}` and the key for an OpenVAIR API Key. OpenVAIR only rewrites model names when needed (see [model mapping](/docs/models#model-mapping)) and never converts protocols (see [Protocols must match](/docs/concepts#protocols-must-match)).

## Authentication

The API Key is read from these headers, in order:

| Header | Typically used by |
| --- | --- |
| `Authorization: Bearer <your API Key>` | OpenAI SDK, most clients |
| `x-api-key: <your API Key>` | Anthropic SDK |
| `x-goog-api-key: <your API Key>` | Gemini SDK |

Your API Key is never forwarded upstream; the channel's own key is used instead.

## Supported endpoints

| Endpoint | Protocol | Model name in |
| --- | --- | --- |
| `POST /v1/chat/completions` | OpenAI Chat Completions | Body `model` |
| `POST /v1/responses` | OpenAI Responses | Body `model` |
| `POST /v1/messages` | Anthropic Messages | Body `model` |
| `POST /v1beta/models/{model}:generateContent` | Gemini | Path |
| `POST /v1beta/models/{model}:streamGenerateContent` | Gemini streaming | Path |
| `GET /v1/models`, `GET /v1beta/models` | Model list | Only models this key can use |

- Streaming is passed through as is. Request bodies are limited to 50 MB.
- Other endpoints return 404 `not_found`, e.g. `/v1/embeddings`, `/v1/messages/count_tokens`, image and audio endpoints.
- Everything except the model lists requires `POST` (otherwise 400); model names can't contain `*`.
- Both model list endpoints return the OpenAI format `{"object": "list", "data": [{"id": …}]}`, so the Gemini SDK's list-models feature can't parse it. Listing models doesn't call any upstream and costs nothing, so it's a quick way to check a key.

## Examples

Set `OPENVAIR_API_KEY` first, and replace the model names with ones your key can use.

```bash OpenAI Chat Completions
curl {{origin}}/v1/chat/completions \
  -H "Authorization: Bearer $OPENVAIR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4.1-mini",
    "messages": [{"role": "user", "content": "Hello"}]
  }'
```

```bash Anthropic Messages
curl {{origin}}/v1/messages \
  -H "x-api-key: $OPENVAIR_API_KEY" \
  -H "anthropic-version: 2023-06-01" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "claude-sonnet-5-5",
    "max_tokens": 1024,
    "messages": [{"role": "user", "content": "Hello"}]
  }'
```

```bash Gemini
curl "{{origin}}/v1beta/models/gemini-2.5-flash:generateContent" \
  -H "x-goog-api-key: $OPENVAIR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"contents": [{"parts": [{"text": "Hello"}]}]}'
```

```bash List available models
curl {{origin}}/v1/models \
  -H "Authorization: Bearer $OPENVAIR_API_KEY"
```

## Error responses

```json Error format
{
  "error": {
    "code": "model_not_allowed",
    "message": "此 Key 无权使用模型 gpt-4o。在 {{origin}}/my-key 查看此 Key 的状态与额度"
  }
}
```

`message` is in Chinese (here: "This key is not allowed to use model gpt-4o. Check this key's status and quota at {{origin}}/my-key"). Except for `invalid_api_key`, it ends with the key lookup page address. All codes and fixes: [Troubleshooting](/docs/troubleshooting#by-error-code).

If an upstream returns an error and no other channel can take over, its status code and response are passed back unchanged.
