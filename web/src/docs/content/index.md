OpenVAIR 把已有的模型渠道（官方 API、中转站、内网服务）汇到一个地址 `{{origin}}`，向工具和同事发放 **API Key**：渠道出错自动切换，每把 Key 单独限额，不保存对话内容。模型费用由供应商直接结算。

```text 一次请求怎么经过 OpenVAIR
你的工具 ──API Key──▶ OpenVAIR ──上游 Key──▶ 官方 API / 中转站 / 内网服务
                        ├─ 检查 Key 的模型权限和额度
                        ├─ 按优先级选渠道，失败换下一个
                        └─ 记录耗时、Token 和费用
```

| 我想… | 看这篇 |
| --- | --- |
| 接好渠道、发出第一把 Key（管理员） | [快速开始](/docs/quickstart) |
| 理解渠道、Key、模型名和协议 | [核心概念](/docs/concepts) |
| 接入 Claude Code、Codex 等本机工具（推荐） | [OpenVAIR Desktop](/docs/desktop) |
| 手动接入 | [Claude Code](/docs/claude-code)、[Codex](/docs/codex)、[Gemini CLI](/docs/gemini-cli)、[其他客户端](/docs/other-clients) |
| 我拿到了一把 Key（无需帐号） | [查看我的 Key](/docs/my-key) |
| 请求报错 | [问题排查](/docs/troubleshooting) |
