## Model rules

A channel's **Model rules** (模型规则) decide which models it provides.

| Format | Meaning | Example |
| --- | --- | --- |
| Exact model name | Only that model | `gpt-4.1-mini` |
| Prefix + `*` | Every model with the prefix, including future ones | `claude-*`, `gemini-2.5-*` |

- `*` may appear once, at the **end** only; a lone `*` or `gpt-*-mini` isn't supported.
- **Case-sensitive**: `gpt-*` doesn't match `GPT-4.1`.
- Press Enter or type a comma to add; text not confirmed with Enter is dropped when you leave the input.
- In the dropdown, **Add the N currently matching models** (添加当前匹配的 N 个模型) adds each model matching now; **Add prefix rule xxx\*** (添加前缀规则 xxx\*) also covers future models with the prefix.

### Forward all

With **Forward all** (全部转发) checked, requests for any model go to this channel unchanged, and the upstream decides whether it works.

> [!WARNING]
> A Forward all channel receives every request of its protocol; with high priority it may grab other channels' models and fail with "model not found". OpenVAIR switches channels when the upstream explicitly says the model doesn't exist, but give it a low priority (a larger number) anyway.

## Model mapping

Rewrites the **public model name** (in the client's request) to the **upstream model name**. Only available after creation: click **Edit** (编辑) and add entries under **Model mapping** (模型映射).

![Setting model rules and model mapping while editing a channel](/docs-img/channel-models.webp)

| Case | Public model name | Upstream model name |
| --- | --- | --- |
| Relay names differ | `claude-sonnet-5-5` | `claude-5.5-sonnet` |
| Stable alias | `team-default` | `gpt-4.1-mini` |
| One name across channels | `fast` | `gpt-4.1-mini` on official, `claude-haiku-4-5` on a relay |

- With one public name across channels, clients keep requesting the same name after failover; the channels must share a protocol.
- Public and upstream names must be exact, no `*`; public names can't repeat within a channel.
- A mapped public name is accepted by the channel even if it isn't in the model rules.
- **Cost is calculated from the upstream model name.**

## Model permissions for keys

The key's **Allowed models** (允许模型; same syntax, **public model names**) and the channel rules must both match:

```text
Request model = claude-sonnet-5-5
   ├─ Key allows it?   Allowed models include claude-*          ✓
   └─ Channel exists?  Anthropic channel rules include claude-* ✓  → sent
```
