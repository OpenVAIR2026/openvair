Start by pasting the failing key into [Check my key](/my-key) (查看我的 Key; the page that errors ending in "在 {{origin}}/my-key 查看此 Key 的状态与额度" point to) to see its status, quota and models.

## By error code

Format: `{"error":{"code":"…","message":"…"}}`. Chinese messages are shown with a translation.

| Status · `code` | `message` | Fix |
| --- | --- | --- |
| 401 `invalid_api_key` | `OpenVAIR Key 无效或已撤销` ("The OpenVAIR key is invalid or has been revoked") | Copy the full key again; check the endpoint is `{{origin}}`; a revoked key needs replacing |
| 401 `key_paused` | `此 OpenVAIR Key 已暂停，请联系 Key 的发放方` ("This OpenVAIR key is paused. Contact whoever issued the key.") | Admin re-enables it |
| 403 `model_not_allowed` | `此 Key 无权使用模型 …` ("This key is not allowed to use model …") | Add the model or a prefix to **Allowed models** (允许模型), or use an allowed model |
| 404 `model_unavailable` | `此 Key 当前没有可以调用模型 … 的渠道` ("This key currently has no channel that can call model …") | See the checklist below |
| 404 `not_found` | not found | See [404 · not found](#404--not-found) |
| 429 `rpm_limit` | `已达到此 Key 每分钟 … 次请求的上限，请稍后重试` ("This key has reached its limit of … requests per minute. Please try again later.") | Retry later, or raise **Requests per minute** (每分钟请求数) |
| 429 `daily_token_limit` | `此 Key 今日的 Token 额度已用完，次日自动恢复，或请发放方重置` ("This key's token quota for today is used up. It resets automatically tomorrow, or ask the issuer to reset it.") | Resets at 08:00 Beijing time; or **Reset today's usage** (重置今日用量), or raise the limit |
| 429 `daily_cost_limit` | `此 Key 今日的费用额度已用完，次日自动恢复，或请发放方重置` ("This key's cost quota for today is used up. It resets automatically tomorrow, or ask the issuer to reset it.") | Same as above |
| 429 `total_cost_limit` | `此 Key 的总费用额度已用完，请发放方提高上限或重置` ("This key's total cost quota is used up. Ask the issuer to raise the limit or reset it.") | Raise **Total cost limit** (总费用上限), or **Reset all usage** (重置全部用量) |
| 400 `bad_request` | `请求必须提供具体模型 ID` ("A concrete model ID is required") | Model name can't be a wildcard such as `gpt-*` |
| 400 `bad_request` | request body model is required | The tool's model setting is empty |
| 400 `bad_request` | request body must be valid JSON | The body is malformed |
| 400 `bad_request` | `渠道映射必须指向具体上游模型` ("Mapping must point to a concrete upstream model") | Set the [model mapping](/docs/models#model-mapping) target to a specific model |
| 400 `bad_request` | gateway endpoints accept POST, except GET /v1/models | Use `POST` (except model listing) |
| 413 `payload_too_large` | request body exceeds 50 MiB | Remove earlier messages or attachments |

Tools such as Claude Code call small models in the background (e.g. Haiku); if `model_not_allowed` appears only occasionally, check those models are allowed.

`model_unavailable` checklist:

1. The channel protocol matches the tool; see [Protocols must match](/docs/concepts#protocols-must-match).
2. The channel's model rules include the model (case-sensitive); if names differ, add a [model mapping](/docs/models#model-mapping).
3. The channel is enabled.
4. If the key doesn't have **Allow all channels** (允许全部渠道), the channel is in its scope.

### 404 · not found

Only [supported endpoints](/docs/api#supported-endpoints) are forwarded. Embeddings, images, audio, token counting (`/v1/messages/count_tokens`) and similar return `not_found`; have the tool connect to the provider directly for those.

### 504 · First-token timeout

Channels produced no output within the [streaming first-token timeout](/docs/monitoring#streaming-first-token-timeout) and no other channel took over; **Channel attempts** (渠道尝试) shows "上游 N 秒内没有输出（首字超时）" ("No output from upstream within N seconds"). For reasoning models, raise the timeout; if the upstream is stuck, add a [fallback channel](/docs/routing).

### Other 4xx or 5xx

When every channel fails, the last channel's error is returned unchanged; see **Channel attempts** in **Request details** (调用明细) for each channel's reason.

| Upstream returns | Fix |
| --- | --- |
| 401 / 403 | Channel's upstream key is invalid; replace it (no failover) |
| 402 | Upstream balance too low |
| 429 | Upstream rate limit; add a [fallback channel](/docs/routing) |
| 5xx | Upstream outage; retry later |
| 502 `upstream request failed` | Can't connect, or no completion within 600 seconds; check the Base URL |

## By symptom

| Symptom | Fix |
| --- | --- |
| Connection test fails | Check the Base URL (follow the relay's docs if it doesn't use `/v1`), protocol and **Test model** (测试模型) |
| `channel URL resolves to a private or reserved network` | Server has `ALLOW_PRIVATE_UPSTREAMS=false`; remove it and restart if you need internal addresses |
| Internal address unreachable | In Docker, `localhost` is the container; use the host's internal IP |
| Channel shows "Failing" (异常) | Click **Check** (检查) for the reason; see [Availability monitoring](/docs/monitoring#availability-monitoring) |
| Requests don't appear in OpenVAIR | The tool isn't using the OpenVAIR address, or the request was rejected outright (quota, permission) and isn't logged |
| Costs don't match the bill | Costs are estimates; see [How cost is calculated](/docs/limits#how-cost-is-calculated). For large gaps, enable a service checkup's metering check |
| Copying a key shows "Not copyable" (不可复制) | Older key stored only as a hash; create a new one |
| "登录失败次数过多，请 15 分钟后再试" ("Too many failed sign-ins, try again in 15 minutes") | Wait 15 minutes; for a forgotten password see [Account settings](/docs/account#forgot-password) |

![A channel marked failing in Channel monitoring](/docs-img/monitor-availability.webp)

### Slow requests

Check which part of **Time breakdown** (耗时分解) in the request details is long:

- **Concurrency queue** (并发排队): the key's **Concurrency limit** (并发上限) is too low.
- **Earlier channel attempts** (前序渠道尝试): an earlier channel failed or hit the [first-token timeout](/docs/monitoring#streaming-first-token-timeout); check **Channel attempts** and adjust channels or priorities.
- **Sent → response headers** (发出 → 响应头) / **Response headers → first output** (响应头 → 首个输出): the upstream is slow; compare first-token times in **Channel performance** (渠道表现) and move faster channels up.

### Signing in sends you back to the sign-in page

The cookie is `Secure` by default (`COOKIE_SECURE=true`), so over HTTP to a non-local address the browser won't send it. Set up HTTPS; only on a trusted internal network, set `COOKIE_SECURE=false` and restart (the startup log warns; never on the public internet). `localhost` is unaffected.

## Still stuck?

Note the **Request record ID** (请求记录 ID) in request details and investigate it with the error message and server logs.
