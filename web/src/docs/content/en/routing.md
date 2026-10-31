## Set up a fallback channel

1. Add two or more channels with **the same protocol** whose model rules include the model.
2. Give the primary a low **Channel priority** (渠道优先级), e.g. `10`, and the fallback a higher one, e.g. `20`.

```text Example
Request claude-sonnet-5-5
  ① Official API (priority 10) ── 429 rate limited ──┐
  ② Relay A (priority 20)      ◀─────────────────────┘ takes over, returns the result
```

Failover stays within one protocol; see [Protocols must match](/docs/concepts#protocols-must-match).

## When OpenVAIR switches channels

| Situation | Switches? |
| --- | --- |
| Connection failure | Yes |
| No complete response within 600 seconds | Yes |
| Streaming request with no output within the **Streaming first-token timeout** (流式首字超时) | Yes |
| Response dropped before any output | Yes |
| 429, 402, 408, 5xx | Yes |
| Upstream explicitly says the model doesn't exist or isn't supported | Yes |
| 401, 403 (upstream key invalid or lacks permission) | No, returned directly |
| Other 4xx (e.g. bad parameters) | No |

![A failover: the first channel returns 429, and the request succeeds on the second channel](/docs-img/request-detail.webp)

If every candidate fails, the last channel's error is returned. Once output has started, a later disconnect can't be failed over. To see which channels a request tried, check **Channel attempts** (渠道尝试) in **Request details** (调用明细).

### Streaming first-token timeout

If a streaming request produces no output (text, reasoning or tool calls) within the limit, OpenVAIR abandons that channel and tries the next; the client only receives the next channel's response.

- Default 100 seconds; adjust per channel in **Channel monitoring** (渠道监控), 5–300 seconds. Independent of the scheduled-check switch. Reasoning models think longer, so don't set it too short.
- On the last candidate there's no switch; OpenVAIR waits for output or the overall 600-second timeout.

## Which channels are candidates

All of these must hold:

- Protocol matches the request.
- Model rules include the model, Forward all is on, or a matching mapping exists.
- Enabled.
- Not excluded by a [service checkup](/docs/monitoring#service-checkup) **Remove** (剔除) policy.
- Within the key's **Allowed channels** (接入渠道).

## Ordering rules

1. **Failing** (异常) channels whose **After marking failing** (判定异常后) is **Move to the end of routing** (移到路由最后, default) go last; with **Only log and alert** (仅记录和告警) they keep their position.
2. Channels given **Lower priority** (降低优先级) by a service checkup go after healthy ones.
3. Lower **Channel priority** goes first.
4. On ties, the earlier-created channel goes first.

Failing channels remain candidates and are still tried when nothing else works, so a monitoring false positive never makes a model fully unavailable.

### When a channel counts as failing

After the set number of failures in a row (default 2); any success resets the count. For what counts and recheck intervals, see [Availability monitoring](/docs/monitoring#judgment-rules).
