## Instance and admin

- Each self-hosted **instance** is one account; all channels, keys, request records and monitoring data belong to it.
- The **admin** is the user created on the `/setup` page with the setup token at first launch, and the only person who can sign in to the console. No public sign-up, no member invitations.
- Everyone else just gets an API key, connects local tools with [OpenVAIR Desktop](/docs/desktop), and checks quota and usage on the [key lookup page](/docs/my-key).

## Channels and API keys

A **channel** is an upstream service OpenVAIR calls: endpoint (Base URL) + upstream key + protocol. See [Upstream channels](/docs/channels). An **API key** is what you give to tools; by default it starts with `openvair_`. See [Create and manage API keys](/docs/keys).

| | Upstream key | API key |
| --- | --- | --- |
| Issued by | The model provider | The admin, in OpenVAIR |
| Lives in | Server only, encrypted with `MASTER_KEY` | Your tools and programs |
| If it leaks | Rotate it with the provider | Revoke just that key |

## Model names

The model name in the request (e.g. `claude-sonnet-5-5`) decides whether the key may use it, which channels can serve it, and the actual name sent upstream. It's passed through unchanged unless rewritten by [model mapping](/docs/models#model-mapping).

## Routing and priority

When several channels serve a model, they're tried by **priority** (lower number first); on rate limits, timeouts or failures the next one is tried. See [Fallback channels and failover](/docs/routing).

```text
                        ┌─ Channel A: Anthropic official (priority 10)
Claude Code ── API key ─┤
  model: claude-…       └─ Channel B: relay service (priority 20) ← takes over when A is rate-limited
```

### Protocols must match

OpenVAIR never converts between protocols, and failover stays within one protocol:

| Endpoint | Goes only to | Common tools |
| --- | --- | --- |
| `/v1/chat/completions`, `/v1/responses` | OpenAI-compatible channels | Codex, Cursor, Cherry Studio, most SDKs |
| `/v1/messages` | Anthropic-compatible channels | Claude Code, Anthropic SDK |
| `/v1beta/models/…` | Gemini-compatible channels | Gemini CLI, Google Gen AI SDK |

So an OpenAI-compatible channel can't handle Claude Code's requests even if it offers Claude models.
