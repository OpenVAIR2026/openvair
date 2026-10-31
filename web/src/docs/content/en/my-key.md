Got an API key? Paste it on the [View my key](/my-key) page to see its status, quota, available models and request history, and to copy setup configs. **No account needed.**

## Opening the page

Go to [{{origin}}/my-key]({{origin}}/my-key), use the link "只有 OpenVAIR Key？查看额度与接入方式" ("Only have an OpenVAIR key? Check quota and setup") below the login form, or follow the link in an error message.

Paste the key and click **View** (查看). Don't tick **Remember this key in this browser** (在此浏览器记住这把 Key) on a shared computer, and click **Clear key** (清除 Key) at the top right when done.

![Paste the key, then click View](/docs-img/my-key-entry.webp)

## Overview

![The My key page: status, quota, available models and setup](/docs-img/my-key.webp)

| Status | Meaning |
| --- | --- |
| Active (正常) | Requests work |
| Paused (已暂停) | Disabled by the issuer; contact them |
| Used up for today (今日已用完) | The page shows when it resets |
| Quota used up (额度已用完) | Total cost used up; the issuer must raise or reset it |

- **Quota**: usage and caps for **Today's tokens** (今日 Token), **Today's cost** (今日费用) and **Total cost** (累计费用), plus requests per minute, the concurrency cap and the daily reset time.
- **Available models**: grouped by protocol; **click a name to copy it**. Claude Code can only use the Anthropic group, Codex only the OpenAI group.
- **Setup**: we recommend entering the server address and key in [OpenVAIR Desktop](/docs/desktop) to write local configs in one click; or pick your tool and click **Copy config** (复制配置; includes the endpoint, full key and model name).

## Usage

The **Usage** (用量) tab shows request count, tokens, cost, average duration, a trend chart and a per-model breakdown for the selected range, plus **Request details** (调用明细; click a row for the timing breakdown). Requests rejected outright (e.g. quota used up) don't appear there.

This page can't change settings or show channel details; ask the issuer for changes.
