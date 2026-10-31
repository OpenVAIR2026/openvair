# Key 持有者自助

## 背景

管理员常把 OpenVAIR 的 API Key 发给没有控制台帐号的人，例如同事、学生或自动化任务的维护者。这些人只有一把 Key，以前只能把它填进客户端，看不到额度和用量；被停用或超额时网关只返回笼统的 401 或 429。

「Key 持有者」不是帐号角色：Key 本身就是凭据，只能读取这把 Key 的信息。这样不需要成员、邀请和角色，与「一个实例一个管理员帐号」的约定一致。

## 持有人字段

迁移 `0075_key_holder.sql` 为 `virtual_api_keys` 增加可空的 `holder` 列，记录这把 Key 发给了谁，只供管理员自己查阅：

- 创建和编辑 Key 时可填写「持有人」；Key 列表在名称下方显示「持有人：…」，搜索框同时匹配名称、持有人和前缀。
- 「复制配置」新建 Key 时不复制名称和持有人。
- 持有人不会通过 `/api/key/*` 返回给持有者。

## 接口

全部用 Key 鉴权，与网关相同（`Authorization: Bearer`、`x-api-key`、`x-goog-api-key`），不读写 cookie，响应带 `Cache-Control: no-store`。代码在 `src/key_holder.rs`。

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| GET | `/api/key/summary` | 帐号名、Key 的 id、名称和前缀、状态、各项上限及当前进度、今日额度重置时间、创建时间、最近调用时间、按协议分组的可用模型 |
| GET | `/api/key/usage?range=&utc_offset_minutes=` | 这把 Key 的总量、趋势和按模型汇总 |
| GET | `/api/key/events?range=&status=&page=&limit=` | 这把 Key 的逐条调用记录，每页最多 100 条 |
| GET | `/api/key/events/{id}` | 单条调用的详情，见 [请求详情与分段计时](request-diagnostics.md) |
| GET | `/api/key/quota-events` | 管理员增加、调整总费用额度或重置用量的记录及备注 |

- **状态**：取值为 `active`、`paused`、`exhausted_daily`、`exhausted_total`。Key 被停用，或所属帐号被停用，都视为 `paused`。
- **已停用的 Key** 仍然可以打开自助页，方便持有者了解原因；已撤销和不存在的 Key 统一返回 401 `invalid_api_key`。
- **可用模型**与网关 `/v1/models` 的计算方式相同：帐号的渠道、Key 的渠道范围和模型规则。
- **id**：持有者本来就有这把 Key，返回 id 不泄露新信息；OpenVAIR Desktop 用它判断本机 Agent 是否在用这把 Key。
- **隐私**：调用明细只包含时间、模型、协议、状态码、耗时、Token、费用和计价状态；不返回渠道、上游模型和价格快照。
- **限流**：同一 IP 15 分钟内失败 30 次后返回 429。Redis 不可用时放行，与登录限流一致。

## 网关错误原因码

网关拒绝请求时返回稳定的 `error.code`。对于可以恢复的情况，消息末尾会附上 `{origin}/my-key` 链接。

| code | 状态 | 场景 |
| --- | --- | --- |
| `invalid_api_key` | 401 | Key 不存在或已撤销（不附链接） |
| `key_paused` | 401 | Key 或其所属帐号被停用 |
| `model_not_allowed` | 403 | 模型不在 Key 的允许范围内 |
| `model_unavailable` | 404 | 没有可以服务该模型的渠道 |
| `rpm_limit` | 429 | 超过每分钟请求数 |
| `daily_token_limit` | 429 | 今日 Token 额度已用完 |
| `daily_cost_limit` | 429 | 今日费用额度已用完 |
| `total_cost_limit` | 429 | 总费用额度已用完 |
| `cost_paused` | 409 | 今日有费用无法确定的请求，按策略暂停 |

在网关里被拒绝的请求不会写入 `usage_events`，所以不会出现在调用明细中，原因只返回给客户端。

## 网页

`/my-key` 是独立页面，不使用控制台布局，登录页有入口「只有 OpenVAIR Key？查看额度与接入方式」。

- Key 默认只存在当前标签页的 `sessionStorage` 中。勾选「在此浏览器记住」后改存 `localStorage`。点击「清除 Key」或 Key 失效时会同时清除这两处。
- 页面顶部有两个标签页，各有独立网址，刷新和分享链接都会停在同一页：
  - **概览**（`/my-key`）：状态和额度、可用模型、接入方式、额度记录。网关错误链接指向这里。
  - **用量**（`/my-key/usage`）：统计、趋势、按模型汇总和调用明细。
- 接入方式复用 `client-snippets.ts`，页面上不显示完整 Key，复制配置时才填入。

## OpenVAIR Desktop

桌面客户端只使用 API Key：首次启动填写服务器地址和 Key，通过 `/api/key/summary` 校验；Key 页显示这里的状态、额度和可用模型，并链接到网页 `/my-key/usage`。详见 [OpenVAIR Desktop 设计](local-client-design.md)。

## 测试

`scripts/test-key-holder.sh` 会启动独立的临时 PostgreSQL 和 Redis，覆盖以下场景：

- 状态、额度和模型；不泄露上游信息；
- 已停用和所属帐号被停用时返回 `key_paused`，撤销后返回 `invalid_api_key`；
- 调用明细只包含本 Key 的请求，并支持状态筛选；
- 网关各类原因码和自助页链接；
- 按 IP 的失败限流。

`scripts/test-model-management.sh` 运行全部数据库测试时也会包含这些用例。
