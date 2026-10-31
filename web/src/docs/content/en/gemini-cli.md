You need an API key that can use `gemini-*` and a **Gemini-compatible** channel (e.g. Base URL `https://generativelanguage.googleapis.com` with a Google AI Studio key); see [Protocols must match](/docs/concepts#protocols-must-match).

> [!TIP]
> We recommend [OpenVAIR Desktop](/docs/desktop): paste the key, pick a model and click **Connect** (接入); disconnecting restores the original config. The manual steps below are for servers, CI, or machines without the client.

## Manual setup

### Set environment variables

**Don't** add `/v1beta` to the address:

```bash macOS / Linux
export GOOGLE_GEMINI_BASE_URL="{{origin}}"
export GEMINI_API_KEY="your API key"
gemini
```

```powershell Windows PowerShell
$env:GOOGLE_GEMINI_BASE_URL = "{{origin}}"
$env:GEMINI_API_KEY = "your API key"
gemini
```

On first launch, choose **Gemini API Key** sign-in, not Google account.

### Make it permanent

Put the variables in `~/.gemini/.env`:

```bash ~/.gemini/.env
GOOGLE_GEMINI_BASE_URL={{origin}}
GEMINI_API_KEY=your API key
```

If it still asks for Google sign-in, set this in `~/.gemini/settings.json`:

```json ~/.gemini/settings.json
{
  "security": {
    "auth": {
      "selectedType": "gemini-api-key"
    }
  }
}
```

## Common issues

| Symptom | Fix |
| --- | --- |
| "此 Key 无权使用模型 …" ("This key is not allowed to use model …") | Add `gemini-*` to the key's **Allowed models** (允许模型) |
| "此 Key 当前没有可以调用模型 … 的渠道" ("This key currently has no channel that can serve model …") | The channel must be **Gemini-compatible** (Gemini 兼容) |

More in [Troubleshooting](/docs/troubleshooting).
