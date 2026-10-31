## 实例和管理员

- 一个自部署**实例**就是一个帐号，渠道、Key、调用记录和监控数据都属于它。
- **管理员**是首次启动时在 `/setup` 页用初始化令牌创建的用户，也是唯一能登录控制台的人。不开放注册，不支持邀请成员。
- 其他人只拿 API Key，用 [OpenVAIR Desktop](/docs/desktop) 一键接入本机工具，在 [Key 查询页](/docs/my-key) 查看额度和用量。

## 渠道和 API Key

**渠道**是 OpenVAIR 调用的上游服务：接口地址（Base URL）+ 上游 Key + 协议，见[上游渠道](/docs/channels)。**API Key** 是发给工具的 Key，默认以 `openvair_` 开头，见[创建和管理 API Key](/docs/keys)。

| | 上游 Key | API Key |
| --- | --- | --- |
| 发放方 | 模型供应商 | 管理员在 OpenVAIR 创建 |
| 存放位置 | 仅服务器，`MASTER_KEY` 加密 | 你的工具和程序 |
| 泄露后 | 到供应商处更换 | 撤销该把，其他不受影响 |

## 模型名

请求中的模型名（如 `claude-sonnet-5-5`）决定：Key 是否有权使用、哪些渠道能提供、发给上游的真实名字。默认原样转发，可用[模型映射](/docs/models#模型映射)改写。

## 路由与优先级

多个渠道提供同一模型时，按**优先级**（数值越小越优先）尝试，限流、超时或故障时换下一个，见[备用渠道与故障切换](/docs/routing)。

```text
                        ┌─ 渠道 A：Anthropic 官方（优先级 10）
Claude Code ── API Key ─┤
  model: claude-…       └─ 渠道 B：中转站（优先级 20）← A 限流时自动接手
```

### 协议必须一致

OpenVAIR 不在协议之间转换，故障切换也只在同协议渠道之间进行：

| 请求接口 | 只发给 | 常见工具 |
| --- | --- | --- |
| `/v1/chat/completions`、`/v1/responses` | OpenAI 兼容渠道 | Codex、Cursor、Cherry Studio、多数 SDK |
| `/v1/messages` | Anthropic 兼容渠道 | Claude Code、Anthropic SDK |
| `/v1beta/models/…` | Gemini 兼容渠道 | Gemini CLI、Google Gen AI SDK |

因此 OpenAI 兼容渠道即使提供 Claude 模型，也接不住 Claude Code 的请求。
