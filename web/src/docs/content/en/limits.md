## What you can limit

| Limit | Description | Resets |
| --- | --- | --- |
| **Requests per minute** (每分钟请求数) | Per calendar minute | Next minute |
| **Concurrency limit** (并发上限) | Requests in flight; extras queue | When earlier requests finish |
| **Daily tokens** (每日 Token) | Input + output, in **millions of tokens** | 08:00 Beijing time (00:00 UTC) |
| **Daily cost** (每日费用) | USD | 08:00 Beijing time (00:00 UTC) |
| **Total cost** (总费用) | Since creation or last reset, USD | Never automatically |

Add the last three via **Add limit** (添加限制) under **Usage limits** (用量限制) when creating or editing a key.

![Adding a daily token limit under Usage limits when creating a key](/docs-img/key-create.webp)

## How cost is calculated

Cost = token usage reported by the upstream × the configured model price ([Usage analytics → Model prices](/usage?prices=1) (使用分析 → 模型单价)).

- Prices match the upstream model name actually called; with [model mapping](/docs/models#model-mapping), the mapped name is priced.
- Time-of-day pricing (e.g. DeepSeek doubles prices at weekday peak) is always estimated at peak.
- Each request stores the price in effect; later changes don't affect old records.
- No price for the model, or no usage returned: no cost, and it doesn't count toward cost limits.
- All costs are **estimates**; the vendor's bill is the source of truth (discounts, cache billing, exchange rates and so on cause differences).

> [!IMPORTANT]
> Quotas are checked **before** a request starts. Requests in progress aren't cut off and cost isn't reserved, so usage can slightly exceed a limit. E.g. with a $5 daily limit and $4.90 used, a large request may reach $5.30.

## When a limit is reached

The client gets a 429 (403 for a model not allowed); codes and messages are in [Troubleshooting](/docs/troubleshooting#by-error-code). To restore now, raise the limit or click **Reset today's usage** (重置今日用量) on the key's row (total cost needs **Reset all usage** (重置全部用量)).

## Check usage

- The **API Keys** (API Key) list shows each key's tokens today, cost today and total cost; yellow at 80%, red at 100%.
- **More actions** (更多操作, ⋯) → **View usage** (查看用量) opens **Usage analytics** filtered to that key.

## Recommended settings

| Scenario | Suggestion |
| --- | --- |
| Your own computer | None, or a generous daily cost |
| Colleagues or friends | Daily + total cost, restricted models |
| CI and scripts | Requests per minute + daily cost |
| Public apps | Every limit |
