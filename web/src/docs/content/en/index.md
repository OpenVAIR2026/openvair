OpenVAIR puts your existing model channels (official APIs, relay services, internal services) behind one URL, `{{origin}}`, and issues **API keys** to tools and teammates: failed channels fail over automatically, each key has its own limits, and conversation content isn't stored. Providers bill you directly for model usage.

```text How a request flows through OpenVAIR
Your tool ──API key──▶ OpenVAIR ──upstream key──▶ Official API / relay service / internal service
                        ├─ Checks the key's model permission and quota
                        ├─ Picks a channel by priority, tries the next on failure
                        └─ Records latency, tokens and cost
```

| I want to… | Read this |
| --- | --- |
| Connect a channel and issue the first key (admin) | [Quick start](/docs/quickstart) |
| Understand channels, keys, model names and protocols | [Core concepts](/docs/concepts) |
| Connect Claude Code, Codex and other local tools (recommended) | [OpenVAIR Desktop](/docs/desktop) |
| Connect manually | [Claude Code](/docs/claude-code), [Codex](/docs/codex), [Gemini CLI](/docs/gemini-cli), [other clients](/docs/other-clients) |
| I was given a key (no account needed) | [Check my key](/docs/my-key) |
| My request returned an error | [Troubleshooting](/docs/troubleshooting) |
