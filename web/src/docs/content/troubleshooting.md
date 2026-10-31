先在 [查看我的 Key](/my-key)（错误信息末尾「在 {{origin}}/my-key 查看此 Key 的状态与额度」所指的页面）粘贴 Key，查看状态、额度和可用模型。

## 按错误码查找

格式：`{"error":{"code":"…","message":"…"}}`。

| 状态 · `code` | `message` | 处理 |
| --- | --- | --- |
| 401 `invalid_api_key` | OpenVAIR Key 无效或已撤销 | 重新完整复制 Key，确认地址是 `{{origin}}`；已撤销需换新 |
| 401 `key_paused` | 此 OpenVAIR Key 已暂停，请联系 Key 的发放方 | 管理员重新启用 |
| 403 `model_not_allowed` | 此 Key 无权使用模型 … | 「允许模型」加入该模型或前缀，或改用可用模型 |
| 404 `model_unavailable` | 此 Key 当前没有可以调用模型 … 的渠道 | 见下方清单 |
| 404 `not_found` | not found | 见 [404 · not found](#404--not-found) |
| 429 `rpm_limit` | 已达到此 Key 每分钟 … 次请求的上限，请稍后重试 | 稍后重试，或调高「每分钟请求数」 |
| 429 `daily_token_limit` | 此 Key 今日的 Token 额度已用完，次日自动恢复，或请发放方重置 | 北京时间 08:00 恢复；或「重置今日用量」、调高上限 |
| 429 `daily_cost_limit` | 此 Key 今日的费用额度已用完，次日自动恢复，或请发放方重置 | 同上 |
| 429 `total_cost_limit` | 此 Key 的总费用额度已用完，请发放方提高上限或重置 | 调高「总费用上限」或「重置全部用量」 |
| 400 `bad_request` | 请求必须提供具体模型 ID | 模型名不能含通配符（如 `gpt-*`） |
| 400 `bad_request` | request body model is required | 工具的模型设置为空 |
| 400 `bad_request` | request body must be valid JSON | 请求体格式错误 |
| 400 `bad_request` | 渠道映射必须指向具体上游模型 | [模型映射](/docs/models#模型映射)目标改为具体模型名 |
| 400 `bad_request` | gateway endpoints accept POST, except GET /v1/models | 除模型列表外只接受 `POST` |
| 413 `payload_too_large` | request body exceeds 50 MiB | 删减历史消息或附件 |

Claude Code 等工具会在后台调用小模型（如 Haiku），偶发 `model_not_allowed` 时检查这些模型是否被允许。

`model_unavailable` 检查清单：

1. 渠道协议与工具匹配，见[协议必须一致](/docs/concepts#协议必须一致)。
2. 渠道模型规则包含该模型（区分大小写）；名称不同时加[模型映射](/docs/models#模型映射)。
3. 渠道已启用。
4. Key 未勾选「允许全部渠道」时，渠道在其范围内。

### 404 · not found

只转发[支持的接口](/docs/api#支持的接口)。向量、图片、音频、Token 计数（`/v1/messages/count_tokens`）等返回 `not_found`，需要时让工具直连供应商。

### 504 · 首字超时

渠道超过[流式首字超时](/docs/monitoring#流式首字超时)无输出且无其他渠道接手，「渠道尝试」显示「上游 N 秒内没有输出（首字超时）」。推理模型可调大首字超时；上游卡住则添加[备用渠道](/docs/routing)。

### 其他 4xx 或 5xx

所有渠道失败时原样返回最后一个渠道的错误，各渠道原因见「调用明细」的「渠道尝试」。

| 上游返回 | 处理 |
| --- | --- |
| 401 / 403 | 渠道上游 Key 失效，更换 Key（不会切换渠道） |
| 402 | 上游余额不足 |
| 429 | 上游限流，加[备用渠道](/docs/routing) |
| 5xx | 上游故障，稍后重试 |
| 502 `upstream request failed` | 连不上上游或 600 秒内未完成，检查 Base URL |

## 按现象查找

| 现象 | 处理 |
| --- | --- |
| 测试连接失败 | 检查 Base URL（非 `/v1` 结构的中转站按其说明）、协议、「测试模型」 |
| `channel URL resolves to a private or reserved network` | 服务器设了 `ALLOW_PRIVATE_UPSTREAMS=false`；需要内网地址时去掉并重启 |
| 内网地址连不上 | Docker 容器内的 `localhost` 是容器本身，改用宿主机内网 IP |
| 渠道显示「异常」 | 点「检查」看原因，见[可用性监控](/docs/monitoring#可用性监控) |
| 请求在 OpenVAIR 无记录 | 工具没用 OpenVAIR 地址；或请求被直接拒绝（额度、权限），不计入明细 |
| 费用与账单不符 | 费用是估算，见[费用是怎么算的](/docs/limits#费用是怎么算的)；差距大时开启服务体检的计量检查 |
| 复制 Key 显示「不可复制」 | 早期 Key 只存了哈希，创建新 Key |
| 「登录失败次数过多，请 15 分钟后再试」 | 等待 15 分钟；忘记密码见[帐号设置](/docs/account#忘记密码) |

![「渠道监控」里被判定异常的渠道](/docs-img/monitor-availability.webp)

### 请求很慢

看调用明细的「耗时分解」：

- **并发排队**长：Key 的「并发上限」太低。
- **前序渠道尝试**长：前面渠道失败或[首字超时](/docs/monitoring#流式首字超时)，看「渠道尝试」并调整渠道或优先级。
- **发出 → 响应头** / **响应头 → 首个输出**长：上游慢，在「渠道表现」对比首字时间，把快的调前。

### 登录后又回到登录页

Cookie 默认带 `Secure`（`COOKIE_SECURE=true`），HTTP 访问非本机地址时浏览器不会发送。推荐配置 HTTPS；仅在可信内网可设 `COOKIE_SECURE=false` 并重启（启动日志会警告，勿用于公网）。`localhost` 访问不受影响。

## 还是解决不了？

记下调用明细中的「请求记录 ID」，结合报错和服务器日志排查。
