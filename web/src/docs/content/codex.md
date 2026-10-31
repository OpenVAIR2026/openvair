需要支持 `/v1/responses` 的 **OpenAI 兼容**渠道（官方 OpenAI API 支持；中转站以其说明为准），以及能用目标模型的 API Key。下文以 `gpt-5-codex` 为例，请换成渠道实际提供的模型名。

> [!TIP]
> 推荐用 [OpenVAIR Desktop](/docs/desktop)：粘贴 Key、选择模型、点「接入」即可，断开时还原原配置。以下手动步骤适用于服务器、CI 或不便安装客户端的场景。

## 手动配置

### 第 1 步：编辑配置

在 `~/.codex/config.toml`（不存在则新建）中加入，地址**需要**以 `/v1` 结尾：

```toml ~/.codex/config.toml
model = "gpt-5-codex"
model_provider = "openvair"

[model_providers.openvair]
name = "OpenVAIR"
base_url = "{{origin}}/v1"
env_key = "OPENVAIR_API_KEY"
wire_api = "responses"
```

### 第 2 步：设置 Key

```bash macOS / Linux
export OPENVAIR_API_KEY="你的 API Key"
codex
```

```powershell Windows PowerShell
$env:OPENVAIR_API_KEY = "你的 API Key"
codex
```

永久生效：把 `export` 行加入 `~/.zshrc` 或 `~/.bashrc`。也可用 Key 的「接入说明」生成配置。

## 验证

执行一个简单任务，在「使用分析」→「调用明细」中确认该请求。

## 常见问题

| 现象 | 处理 |
| --- | --- |
| 404 或接口不存在 | 渠道不支持 Responses 接口，更换渠道或向中转站确认 |
| 每轮卡住，报 `workspace routing discovery timed out` | ChatGPT 帐号登录时每轮都访问 chatgpt.com。运行 `codex logout`，或确保能访问 chatgpt.com |
| 「此 Key 无权使用模型 …」 | 将 `model` 加入 Key 的「允许模型」 |

更多见[问题排查](/docs/troubleshooting)。
