Any OpenAI-compatible, Anthropic Messages or Gemini tool with a custom endpoint setting can connect; the channel protocol must match the tool (see [Protocols must match](/docs/concepts#protocols-must-match)).

> [!TIP]
> Qwen Code, Kimi CLI, OpenCode, ZCode and DeepSeek Harness can be connected in one click with [OpenVAIR Desktop](/docs/desktop): paste the key, pick a model and click **Connect** (接入); disconnecting restores the original config. This page covers manual setup for other tools and SDKs.

## What to fill in

| Setting | Value |
| --- | --- |
| Endpoint / Base URL / API Host | OpenAI-compatible: `{{origin}}/v1`; Anthropic and Gemini: `{{origin}}` |
| API Key | Your API key |
| Model | A model your key can use; click to copy on the [key lookup page](/my-key) |

If the tool appends `/v1` itself, enter just `{{origin}}`; on 404, try adding or removing the trailing `/v1`.

## CC Switch

Add a custom provider on the Claude, Codex or Gemini page, save, and switch to OpenVAIR. **On the Codex page, end the request URL with `/v1`.**

```text CC Switch
Provider name: OpenVAIR
API Key: your API key
Request URL: {{origin}}
```

## Chat clients

Cherry Studio, ChatBox, LobeChat, Open WebUI and similar: add an "OpenAI-compatible" or "Custom" provider and fill in the table above. Add models by hand or click "fetch model list" to get the key's models.

## OpenAI SDK

```python Python
from openai import OpenAI

client = OpenAI(
    base_url="{{origin}}/v1",
    api_key="your API key",
)

response = client.chat.completions.create(
    model="gpt-4.1-mini",  # Example; use a model your key can access
    messages=[{"role": "user", "content": "Hello"}],
)
print(response.choices[0].message.content)
```

```javascript Node.js
import OpenAI from 'openai'

const client = new OpenAI({
  baseURL: '{{origin}}/v1',
  apiKey: process.env.OPENVAIR_API_KEY,
})

const response = await client.chat.completions.create({
  model: 'gpt-4.1-mini', // Example; use a model your key can access
  messages: [{ role: 'user', content: 'Hello' }],
})
console.log(response.choices[0].message.content)
```

## Anthropic SDK

**Don't** add `/v1` to `base_url`:

```python Python
from anthropic import Anthropic

client = Anthropic(
    base_url="{{origin}}",
    api_key="your API key",
)

message = client.messages.create(
    model="claude-sonnet-5-5",
    max_tokens=1024,
    messages=[{"role": "user", "content": "Hello"}],
)
print(message.content[0].text)
```

## Other frameworks

LangChain, LlamaIndex, the Vercel AI SDK and similar: set the "custom Base URL" parameter. For raw HTTP, see the [API reference](/docs/api).

> [!WARNING]
> Don't hard-code your API key or commit it to Git; use an environment variable or secrets manager.