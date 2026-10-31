You need an **OpenAI-compatible** channel that supports `/v1/responses` (the official OpenAI API does; for relay services, check their docs) and an API key that can use your model. The examples use `gpt-5-codex`; replace it with a model your channel serves.

> [!TIP]
> We recommend [OpenVAIR Desktop](/docs/desktop): paste the key, pick a model and click **Connect** (接入); disconnecting restores the original config. The manual steps below are for servers, CI, or machines without the client.

## Manual setup

### Step 1: Edit the config

Add this to `~/.codex/config.toml` (create it if needed); the address **must** end with `/v1`:

```toml ~/.codex/config.toml
model = "gpt-5-codex"
model_provider = "openvair"

[model_providers.openvair]
name = "OpenVAIR"
base_url = "{{origin}}/v1"
env_key = "OPENVAIR_API_KEY"
wire_api = "responses"
```

### Step 2: Set the key

```bash macOS / Linux
export OPENVAIR_API_KEY="your API key"
codex
```

```powershell Windows PowerShell
$env:OPENVAIR_API_KEY = "your API key"
codex
```

To make it permanent, add the `export` line to `~/.zshrc` or `~/.bashrc`. The key's **Setup guide** (接入说明) can also generate this config.

## Check that it works

Run a simple task, then find the request in **Usage analytics** (使用分析) → **Request details** (调用明细).

## Common issues

| Symptom | Fix |
| --- | --- |
| 404 or "endpoint doesn't exist" | The channel doesn't support Responses; switch channels or ask your relay service |
| Every turn hangs with `workspace routing discovery timed out` | When signed in with ChatGPT, Codex contacts chatgpt.com every turn. Run `codex logout`, or make chatgpt.com reachable |
| "此 Key 无权使用模型 …" ("This key is not allowed to use model …") | Add `model` to the key's **Allowed models** (允许模型) |

More in [Troubleshooting](/docs/troubleshooting).
