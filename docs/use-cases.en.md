An OpenVAIR instance serves one person or one team and has a single admin account. Members don't sign up; quota is allocated per **key**, and each provider bills you directly.

| You are… | Pain point | Approach |
| --- | --- | --- |
| [A small team](#small-teams) | Overspending, leaked keys | One key per person, own limits |
| [A research group](#research-groups) | Tight budget, student turnover | Keys per person or project; revoke on graduation |
| [An individual developer](#individual-developers) | Many tools; a channel failure stops work | One URL, automatic failover |
| [An app or backend developer](#app-and-backend-developers) | Single provider | Failover, request details |
| [A member with a key](#members-with-a-key) | How to use it, what's left | No account; check quota any time |

## Small teams

Your team uses agents like Claude Code and Codex. You own the budget and don't want to hand out provider keys.

1. [Deploy OpenVAIR](/deploy) and connect the company's channels.
2. [Create a key](/docs/keys) per member, put their name in **Holder** (持有人), and set [limits](/docs/limits).
3. Send them the URL and key. They paste it into [OpenVAIR Desktop](/download) and click **Connect** (接入).

- Going over only limits that person's key; daily caps reset the next day, the total cost cap must be raised manually.
- On offboarding or a leak, [disable](/docs/keys#disable-reset-and-revoke) (reversible) or revoke (permanent).
- Filter usage analytics by key; members check their own quota on the [key lookup page](/docs/my-key).
- An [alert email](/docs/account#alert-emails) goes out when a channel stays unavailable.

## Research groups

A PI manages the budget, maybe with internal models or lab GPUs; students come and go.

1. Deploy on a lab server. For internal use only, [access it by LAN address](/deploy#choosing-a-deployment-method).
2. [Add](/docs/channels) official APIs, relays and internal services as channels; internal as primary, external as fallback.
3. [Create keys](/docs/keys) per person or project with a cost cap and allowed models.
4. Students connect with [OpenVAIR Desktop](/download), or point the SDK Base URL at OpenVAIR (see [Other clients and SDKs](/docs/other-clients)).

- See cost per key, for each person and project; revoke on graduation, since students never hold upstream keys.
- When the internal service is busy or down, requests go to external channels.
- Request bodies are never stored. See [Data and privacy](/docs/privacy).

## Individual developers

You use Claude Code, Codex and Gemini CLI on several devices, with several model sources.

1. [Deploy OpenVAIR](/deploy) on an always-on machine.
2. [Add](/docs/channels) every source as a channel; give the primary one a smaller priority number.
3. [Create a key](/docs/keys) per device, e.g. "MacBook".
4. Configure agents in one click with [OpenVAIR Desktop](/docs/desktop), or manually via [tool setup](/docs/claude-code).

- Switching providers only changes channels; on rate limits or failures, requests [fail over](/docs/routing).
- Provider keys stay off devices; revoke a lost device's key.

## App and backend developers

Your product, internal tools or CI call models and need fault tolerance and per-app tracking.

1. Create a key per app or environment (production, staging, CI) with model, rate and concurrency limits as needed.
2. Point the SDK Base URL and key at OpenVAIR. See [Other clients and SDKs](/docs/other-clients) and the [API reference](/docs/api).
3. Configure several same-protocol channels for a model and [set priorities](/docs/routing).

- Keep the official SDKs; switching providers needs no code changes.
- Rate limits, 5xx, connection failures and [streaming first-token timeouts](/docs/routing#streaming-first-token-timeout) fail over automatically; the client never sees the failed attempt.
- Request details record latency, time to first token, tokens and every channel attempt.

## Members with a key

1. Install [OpenVAIR Desktop](/download) and enter the server URL and key to configure agents in one click, or follow [tool setup](/docs/claude-code) manually.
2. Check available models, remaining quota and recent usage on the [key lookup page](/docs/my-key).

No sign-up. Error messages end with the key lookup page URL; see [Troubleshooting](/docs/troubleshooting) for common errors.
