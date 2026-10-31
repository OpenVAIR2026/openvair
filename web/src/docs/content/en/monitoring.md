**Channel monitoring** (渠道监控) runs two kinds of active checks: **availability monitoring** checks whether a channel is reachable, and **service checkups** check whether a channel + model pair is downgraded, tampered with, or over-reporting usage. Active checks send real requests with the channel's upstream key, so they **cost a small amount**; each setting shows the estimate. Free [passive checks](#passive-checks) analyze real requests.

## Availability monitoring

Scheduled requests ask for 1 output token; real request outcomes count too. New channels have it **on by default**, every 30 minutes. Click **Check now** (立即检查) any time, click the ⓘ next to a status to see why it failed, and use **Last 12** to tell occasional blips from a real outage.

![Availability monitoring: the last 12 check results, with failing channels flagged](/docs-img/monitor-availability.webp)

### Judgment rules

- **Counts as a failure**: a scheduled check without a successful response (including upstream key rejection, 401–403); a real request with upstream 5xx, timeout, connection failure or first-token timeout. 429, 402 and other 4xx don't count.
- **Marked failing**: consecutive failures reach the set count; any success resets it. With scheduled checks off, only manual checks change the status.
- **Inconclusive**: upstream 429, upstream doesn't offer the test model, check not sent. Shown only under **Last check**; doesn't change the verdict. If, after a channel is marked failing, 2× the check interval (at least 1 hour) passes with only inconclusive results and no new failures, the verdict expires and normal routing resumes.
- **Test model**: if unset, a model that succeeded in real requests on this channel in the last 7 days; otherwise the first model by name. If the upstream rejects the model you set, the check retries with a recently successful one and shows a yellow ⚠; update the test model and the ⚠ clears.
- **Rechecks**: 10 minutes after a failure; while failures continue, 10, 20, 40 minutes…, capped at the check frequency.
- **When failing**: by default **moved to the end of routing** (still used if nothing else works), or **Only log and alert, don't affect routing** (仅记录和告警，不影响路由). See [Ordering rules](/docs/routing#ordering-rules). Recovers on the next success.

### Settings

**Channel monitoring → Availability monitoring** (渠道监控 → 可用性监控) → **Settings** (设置) on the channel's row:

![Availability settings for a single channel](/docs-img/monitor-availability-settings.webp)

| Setting | Description |
| --- | --- |
| Enable scheduled availability checks (开启定时可用性检查) | When off, manual checks still work; real requests stop counting |
| Check frequency (检查频率) | 5 minutes–24 hours |
| Consecutive failures before marking failing (连续失败几次后判定异常) | 1–5, default 2 |
| After marking failing (判定异常后) | Move to end of routing (default) or only log and alert |
| Streaming first-token timeout (seconds) (流式首字超时（秒）) | 5–300, default 100 |

### Streaming first-token timeout

With no output before the timeout, OpenVAIR switches to the next channel; this always applies. For the behavior, see [Fallback channels and failover](/docs/routing#streaming-first-token-timeout). Don't set it too short for reasoning models.

## Service checkup

Sends random questions to one model on one channel:

![Service checkup: the monitor list and the results of each run](/docs-img/monitor-quality.webp)

| Check | What it checks |
| --- | --- |
| Answer integrity | Repeat a random passphrase, do a random addition |
| Cache replay | Not a stale answer from an earlier request |
| Metering check | Inflated usage or a hidden system prompt |
| Tool calls | Function calling works |
| Output truncation | Output isn't cut off early |
| Streaming output | Event format, completeness, usage reporting |

A failed answer integrity check fails the run outright; the rest are judged by pass rate. Checks refused by a safety policy or content filter are marked incomplete, not failed.

The metering check compares exactly against a local count for Claude Opus 4.8–5.5 (Messages API) and GPT-5.6 Sol, GPT-6 Astra, GPT-6.1 Sol (Chat Completions or Responses API): reported input tokens 30 tokens or 5% over (whichever is larger) is failing. Other combinations (e.g. Claude via the OpenAI protocol) are estimated: fixed overhead above 100 tokens is flagged as a suspected hidden system prompt, proportional over-reporting is caught only when clearly off, and results are labeled **Rough check only**.

### Add a checkup

1. **Channel monitoring → Service checkup** (渠道监控 → 服务体检) → add a monitor.
2. Choose channel and model (can't be changed later).
3. Set the scheduled check frequency (15 minutes–24 hours).
4. Choose **Routing impact** (路由影响):

| Option | Effect |
| --- | --- |
| Observe only (仅观察, default) | Record only |
| Lower priority when failing (异常时降低优先级) | After healthy channels, still a fallback |
| Remove when failing (异常时剔除) | Not used to route this model |

A result becomes **Confirmed failing**, and the routing policy applies, only after consecutive failures reach the **failure confirmation count** (失败确认次数, default 2). Results stay valid for 2× the check interval, then normal routing resumes.

### Long-context test

Start a **Long-context test** (长上下文检测) manually from the checkup details. It hides a passphrase at a random position in a long text and steps through 8k → 32k → 128k to find the context length the channel really supports.

> [!WARNING]
> One run can send well over 100,000 input tokens, so it's expensive. The estimate is shown before you start.

## Passive checks

OpenVAIR analyzes the last 24 hours of real requests for **metering anomalies** such as over-reported usage and hidden system prompts. No extra requests, no cost, no routing impact.

If the model has a service checkup, it runs early to confirm before an [alert email](/docs/account#alert-emails) goes out; checks that passed a checkup in the last 24 hours withdraw matching warnings. Over-reported usage can't be confirmed by a checkup, so those warnings are sent directly.
