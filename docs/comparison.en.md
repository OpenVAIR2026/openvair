[中文](comparison.md) · **English**

# How OpenVAIR compares to similar projects

OpenVAIR covers only teams sharing model channels, in exchange for less setup and fewer steps. Other projects change quickly; the tables only sketch their positioning, so check their latest docs.

## Similar projects

| Type | Examples | Main use |
|---|---|---|
| API distribution panels | one-api, new-api | Aggregate channels, distribute quota, top-ups |
| General LLM gateways | LiteLLM Proxy, Portkey, Higress / Kong AI plugins | Unified API, routing and observability for apps |
| Local config switchers | cc-switch, claude-code-router, etc. | Switch an agent's upstream on your machine |
| Hosted routing services | OpenRouter, etc. | No deployment, pay as you go |

## What OpenVAIR does and doesn't do

| | OpenVAIR | Notes |
|---|---|---|
| Self-hosted | ✓ | Single tenant, one admin |
| One-click agent setup in the desktop client | ✓ | Original config restored on disconnect |
| Per-key quota, rate, concurrency and models | ✓ | Daily or lifetime token and cost limits |
| Self-service usage for key holders | ✓ | `/my-key` page, no account |
| Automatic channel failover | ✓ | By priority; failing channels cool down |
| Channel availability and quality monitoring | ✓ | Can lower priority or pull a channel |
| Per-request diagnostics | ✓ | Failover path and timings; no bodies stored |
| Multiple instances / HA | ✗ | Single instance only |
| SSO, members and roles | ✗ | |
| Top-ups, billing and resale | ✗ | Out of scope |
| Usage per agent | ✗ | Tracked per key |

## Which to choose

- **Your team shares upstreams and wants agents to just work**: OpenVAIR.
- **Just you, switching upstreams locally**: a local config switcher.
- **Your app needs many providers or multiple instances**: a general LLM gateway.
- **You resell or distribute quota**: an API distribution panel.
- **You don't want to deploy**: a hosted routing service.
