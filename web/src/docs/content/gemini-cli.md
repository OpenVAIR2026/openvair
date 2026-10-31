需要能用 `gemini-*` 的 API Key，以及 **Gemini 兼容**渠道（如 Base URL `https://generativelanguage.googleapis.com` + Google AI Studio Key），见[协议必须一致](/docs/concepts#协议必须一致)。

> [!TIP]
> 推荐用 [OpenVAIR Desktop](/docs/desktop)：粘贴 Key、选择模型、点「接入」即可，断开时还原原配置。以下手动步骤适用于服务器、CI 或不便安装客户端的场景。

## 手动配置

### 设置环境变量

地址**不要**加 `/v1beta`：

```bash macOS / Linux
export GOOGLE_GEMINI_BASE_URL="{{origin}}"
export GEMINI_API_KEY="你的 API Key"
gemini
```

```powershell Windows PowerShell
$env:GOOGLE_GEMINI_BASE_URL = "{{origin}}"
$env:GEMINI_API_KEY = "你的 API Key"
gemini
```

首次启动时登录方式选 **Gemini API Key**，不要选 Google 帐号登录。

### 永久生效

写入 `~/.gemini/.env`：

```bash ~/.gemini/.env
GOOGLE_GEMINI_BASE_URL={{origin}}
GEMINI_API_KEY=你的 API Key
```

仍要求 Google 帐号登录时，在 `~/.gemini/settings.json` 中指定：

```json ~/.gemini/settings.json
{
  "security": {
    "auth": {
      "selectedType": "gemini-api-key"
    }
  }
}
```

## 常见问题

| 现象 | 处理 |
| --- | --- |
| 「此 Key 无权使用模型 …」 | Key 的「允许模型」加入 `gemini-*` |
| 「此 Key 当前没有可以调用模型 … 的渠道」 | 渠道协议须为「Gemini 兼容」 |

更多见[问题排查](/docs/troubleshooting)。
