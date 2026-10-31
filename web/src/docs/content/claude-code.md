需要一把能用 `claude-*` 的 API Key，以及模型规则包含 `claude-*` 的 **Anthropic 兼容**渠道（见[协议必须一致](/docs/concepts#协议必须一致)）。

> [!TIP]
> 推荐用 [OpenVAIR Desktop](/docs/desktop)：粘贴 Key、选择模型、点「接入」即可，断开时还原原配置。以下手动步骤适用于服务器、CI 或不便安装客户端的场景。

> [!WARNING]
> Claude Code 默认请求 `claude-sonnet-5-5`、`claude-opus-5-5`，后台任务使用 `claude-haiku-4-5`。渠道模型名不同时请配置[模型映射](/docs/models#模型映射)。

## 手动配置

### 环境变量

在同一个终端中设置并启动（地址**不要**加 `/v1`）：

```bash macOS / Linux
export ANTHROPIC_BASE_URL="{{origin}}"
export ANTHROPIC_AUTH_TOKEN="你的 API Key"
claude
```

```powershell Windows PowerShell
$env:ANTHROPIC_BASE_URL = "{{origin}}"
$env:ANTHROPIC_AUTH_TOKEN = "你的 API Key"
claude
```

### 永久生效

合并到 `~/.claude/settings.json` 的 `env` 中，保留其他设置：

```json ~/.claude/settings.json
{
  "env": {
    "ANTHROPIC_BASE_URL": "{{origin}}",
    "ANTHROPIC_AUTH_TOKEN": "你的 API Key"
  }
}
```

## 指定模型

运行 `claude --model claude-opus-5-5`，或在 Claude Code 中输入 `/model`。模型须在 Key 的允许范围内且有渠道提供。

## 验证

发送一条消息，然后在「使用分析」→「调用明细」中确认该请求，协议为 Anthropic。

![「调用明细」：协议、渠道、耗时和用量](/docs-img/usage-requests.webp)

## 常见问题

| 现象 | 处理 |
| --- | --- |
| 「此 Key 无权使用模型 …」 | Key 的「允许模型」加入 `claude-*` |
| 「此 Key 当前没有可以调用模型 … 的渠道」 | 检查 Anthropic 兼容渠道已启用且规则包含该模型 |
| 请求未经过 OpenVAIR | 在设置变量的终端启动；登录过官方帐号先 `/logout`；删除 `ANTHROPIC_API_KEY` 变量 |

更多见[问题排查](/docs/troubleshooting)。
