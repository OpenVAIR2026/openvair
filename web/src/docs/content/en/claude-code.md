You need an API key that can use `claude-*` and an **Anthropic-compatible** channel whose model rules include `claude-*` (see [Protocols must match](/docs/concepts#protocols-must-match)).

> [!TIP]
> We recommend [OpenVAIR Desktop](/docs/desktop): paste the key, pick a model and click **Connect** (接入); disconnecting restores the original config. The manual steps below are for servers, CI, or machines without the client.

> [!WARNING]
> Claude Code requests `claude-sonnet-5-5` and `claude-opus-5-5` by default; background tasks use `claude-haiku-4-5`. If your channel uses other names, add a [model mapping](/docs/models#model-mapping).

## Manual setup

### Environment variables

Set the variables and start in the same terminal (**don't** add `/v1` to the address):

```bash macOS / Linux
export ANTHROPIC_BASE_URL="{{origin}}"
export ANTHROPIC_AUTH_TOKEN="your API key"
claude
```

```powershell Windows PowerShell
$env:ANTHROPIC_BASE_URL = "{{origin}}"
$env:ANTHROPIC_AUTH_TOKEN = "your API key"
claude
```

### Make it permanent

Merge this into `env` in `~/.claude/settings.json`, keeping other settings:

```json ~/.claude/settings.json
{
  "env": {
    "ANTHROPIC_BASE_URL": "{{origin}}",
    "ANTHROPIC_AUTH_TOKEN": "your API key"
  }
}
```

## Choose a model

Run `claude --model claude-opus-5-5`, or use `/model` inside Claude Code. The key must allow the model and a channel must serve it.

## Check that it works

Send a message, then find the request in **Usage analytics** (使用分析) → **Request details** (调用明细) with protocol Anthropic.

![Request details: protocol, channel, duration and usage](/docs-img/usage-requests.webp)

## Common issues

| Symptom | Fix |
| --- | --- |
| "此 Key 无权使用模型 …" ("This key is not allowed to use model …") | Add `claude-*` to the key's **Allowed models** (允许模型) |
| "此 Key 当前没有可以调用模型 … 的渠道" ("This key currently has no channel that can serve model …") | Check an Anthropic-compatible channel is enabled and its rules include the model |
| Requests don't go through OpenVAIR | Start in the terminal where you set the variables; `/logout` if signed in with an Anthropic account; remove `ANTHROPIC_API_KEY` |

More in [Troubleshooting](/docs/troubleshooting).
