## Overview

The home page after signing in (运行概览):

![Overview](/docs-img/overview.webp)

- **Status**: channels needing attention, and how many lack availability monitoring.
- **Request stats**: volume, success rate, failures, average first-token latency. **View failed requests** (查看失败调用) jumps to the failures.
- **Recent requests**: the last 5 requests, optionally failures only.
- Right after installation, a **Get started** (开始使用) card covers adding a channel, creating a key and connecting a client.

## Usage analytics

![Usage analytics: requests and cost by time, model and channel](/docs-img/usage-analytics.webp)

**Usage analytics** (使用分析): filter by **channel**, **API Key** and **time range** (last 24 hours, 7 days, 30 days, all); charts can show request count, token count, cost or output performance.

| Metric | Meaning |
| --- | --- |
| **Total requests** | Requests and success share |
| **Total tokens** | Input plus output |
| **Cache rate** | Share of input tokens that hit the upstream prompt cache |
| **Average duration** | Average time, plus time to first token |
| **Time to first token** | From sending the request to the first output |
| **Output speed** | Output tokens per second; successful streaming requests only, excluding those under 32 output tokens |

- **Channel performance**: compares channels by requests, success rate, tokens, cache rate, estimated cost, latency and output speed. Failures are split into **Request errors** (parameters or auth), **Service errors** (upstream or gateway) and **Other unsuccessful**.

- **Cost**: only priced requests; unpriced models are flagged separately and their chart periods shown in amber. See [Limits and cost](/docs/limits#how-cost-is-calculated).

## Request details

The **Request details** (调用明细) list at the bottom shows every request; filter by channel, API Key, model, protocol and status, or export CSV. No conversation content is stored, and requests rejected outright (e.g. quota used up) don't appear. Click a row for details:

![Request details: timing breakdown and each channel attempt](/docs-img/request-detail.webp)

- **Timing breakdown**: **Concurrency queue** → **Earlier channel attempts** → for the last channel, **Sent → response headers** and **Response headers → first output** → **First output → end**.
- **Channel attempts**: channels tried in order, time spent and failure reason (upstream error, connection or send failure, connection or response timeout, first-token timeout (switched channel), interrupted response, client disconnect).
- **Usage and cost**, and the **Request record ID** (include it when reporting a problem).

For reading the timing breakdown of a slow request, see [Troubleshooting](/docs/troubleshooting#slow-requests).
