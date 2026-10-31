## Add a channel

1. **Upstream channels** (上游渠道) → **Add channel** (添加渠道).
2. **Channel source** (渠道来源): for an official service, choose **Official API key** (官方 API Key) and pick the vendor (protocol and Base URL are filled in); otherwise choose **Third-party API key** (第三方 API Key).
3. Choose a **protocol** and fill in the form.
4. Click **Test connection** (测试连接), then **Create channel** (创建渠道) once it passes.

![The add channel form](/docs-img/channel-create.webp)

> [!IMPORTANT]
> The protocol can't be changed after creation; if it's wrong, delete and re-add the channel. Choose by **the tool you use**, not the model; see [Protocols must match](/docs/concepts#protocols-must-match). If a relay service supports several protocols, add one channel per protocol with the same upstream key.

### Form fields

| Field | Description |
| --- | --- |
| Channel name (渠道名称) | For telling channels apart |
| Base URL | Trailing `/v1` optional |
| Upstream API key (上游 API Key) | Stored encrypted; no client can retrieve it |
| Channel priority (渠道优先级) | Lower goes first; default 100. See [failover](/docs/routing) |
| Concurrency limit (并发上限) | Optional. Requests in flight through this channel, streams included; blank means no limit |
| Model rules (模型规则) | See [Model names and mapping](/docs/models) |
| Test model (测试模型) | For tests and health checks; blank picks one |

| Provider | Base URL |
| --- | --- |
| OpenAI | `https://api.openai.com` |
| Anthropic | `https://api.anthropic.com` |
| Google Gemini | `https://generativelanguage.googleapis.com` |
| Relay service | The address in its docs, e.g. `https://api.example.com/v1` |
| Local or internal service | e.g. `http://192.168.1.20:8000` |

`localhost`, private IPs and domains resolving to private addresses are allowed by default. If the server sets `ALLOW_PRIVATE_UPSTREAMS=false`, they're rejected with `channel URL resolves to a private or reserved network`.

### Test connection

Sends a real request asking for 1 output token; the cost is tiny. A result is valid for 14 minutes; after changing the address, key, model rules or test model, test again.

## Fetch upstream models

**Fetch upstream models** (获取上游模型) next to **Model rules** adds chat models from the upstream's list that aren't in your rules yet (embedding, image and similar models are skipped); existing rules stay. With **Forward all** (全部转发) checked, rules aren't changed and results only serve as test model candidates.

- OpenAI compatible and Anthropic read `GET /v1/models`; Gemini reads `GET /v1beta/models`.
- Some Anthropic relay services return 404; enter names manually or check **Forward all**.

## Check channel status

![The channel list: health, model coverage and priority at a glance](/docs-img/channels-list.webp)

| Health (健康状态) | Meaning |
| --- | --- |
| Healthy (健康) | Routed normally |
| Failing (异常) | Consecutive failures hit the set count; moved to the end by default; one success restores it. Reason shown under the status |
| Pending check (待检查) | Settings just changed; not checked yet |
| Test model needed (需测试模型) | Only prefix rules or Forward all; set a test model |
| Disabled (已停用) | Disabled manually |

Channels with a concurrency limit show `in flight / limit` in the **Concurrency** (并发) column, refreshed every 3 seconds. A full channel is passed over for the next candidate; if it's the last one, the request waits up to that channel's first-token timeout (default 100 seconds), then returns 503 `channel_full`, shown as 渠道并发已满 in request details.

Click **Check** (检查) to check now; new channels are checked every 30 minutes by default. For judgment rules, frequency and failure count, see [Availability monitoring](/docs/monitoring#availability-monitoring).

## Edit, disable and delete

- **Edit** (编辑): change the name, address, key, priority, model rules and [model mapping](/docs/models#model-mapping). Leave the upstream key blank to keep the saved one.
- **Disable / Enable**: click the power icon. A disabled channel isn't routed; settings are kept. Good for temporary outages.
- **Delete**: the channel, its upstream key and monitoring settings are deleted permanently; past request records stay.
