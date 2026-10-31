An API key is the credential tools use to call OpenVAIR, each with its own permissions and quota. For personal use, several devices can share one key; one key per device, person or purpose lets you see usage separately and revoke one alone.

## Create a key

1. **API Keys** (API Key) → **Create API Key** (创建 API Key).
2. Fill in the form below and click **Create API Key**.
3. In the panel, click **Copy full API Key** (复制完整 API Key). The panel also shows setup for each tool.

![The Create API Key form](/docs-img/key-create.webp)

| Field | Description | Default |
| --- | --- | --- |
| **Key name** (Key 名称) | Its use, e.g. "MacBook", "CI" | — |
| **Holder (optional)** (持有人（可选）) | Who has it, for search | Empty |
| **Group** (分组) | Filter the list by group | No group |
| **Channels** (接入渠道) | With **Allow all channels** (允许全部渠道), new channels are included | All channels |
| **Allowed models** (允许模型) | Exact names, prefixes (e.g. `claude-*`) or **Allow all models** (允许全部模型) | Required, no default |
| **Requests per minute** (每分钟请求数) | Empty means no limit | 60 |
| **Concurrency limit** (并发上限) | Up to 100 | 4 |
| **Usage limits** (用量限制) | Daily tokens, daily cost, total cost | None |

For what each limit means, see [Limits and cost](/docs/limits).

## Copy the full key

Click the copy button next to the key prefix (e.g. `openvair_AbC…`) in the list; every copy is recorded in the [Activity log](/docs/account#activity-log). A key marked **Not copyable** (不可复制) is older and only its hash was saved; it can't be recovered, so create a new one.

## See setup instructions

Holders are best served by [OpenVAIR Desktop](/docs/desktop): paste the key to connect local tools in one click. For manual setup, open **More actions** (更多操作, ⋯) → **Setup guide** (接入说明) and pick your tool to see a config with the address and model names filled in; **Copy config** (复制配置) adds the full key.

![The Setup guide panel: per-client configuration, with a built-in connection test](/docs-img/key-connect.webp)

**Test connection** (测试连接) in the panel only lists the key's models; it doesn't call any upstream and costs nothing.

## Disable, reset and revoke

All in the **More actions** menu.

| Action | Effect | Reversible? |
| --- | --- | --- |
| **Disable** (停用) | Pauses the key; requests return "paused" | Yes, re-enable any time |
| **Reset today's usage** (重置今日用量) | Today's tokens and cost restart; cumulative unchanged | — |
| **Reset all usage** (重置全部用量) | Daily and cumulative counting restart | — |
| **Revoke** (撤销) | Permanently invalidates the key and removes it from the list | No |

Resetting doesn't delete request records.

![The More actions menu](/docs-img/key-more-menu.webp)

> [!WARNING]
> If a key may have leaked, **revoke** it and create a new one. Disabling only pauses it; re-enabled, the old key works again.

## Key status

| Status | Meaning |
| --- | --- |
| **Enabled** (启用) | Working normally |
| **Disabled** | Disabled manually |
| **Limit reached** (已达上限) | New requests rejected; daily limits reset at 08:00 Beijing time the next day, the total cost limit never resets on its own. Raise the limit or reset usage to restore now |

![The API Keys list: usage, cost and rate limits for each key](/docs-img/keys-list.webp)

## Custom key prefix

Default `openvair_`. Change it under **Account settings → Connection info → API Key prefix** (帐号设置 → 连接信息 → API Key 前缀); rules in [Account settings](/docs/account#api-key-prefix).
