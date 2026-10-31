支持自定义接口地址的 OpenAI 兼容、Anthropic Messages 或 Gemini 工具均可接入，渠道协议须与工具一致（见[协议必须一致](/docs/concepts#协议必须一致)）。

> [!TIP]
> Qwen Code、Kimi CLI、OpenCode、ZCode 和 DeepSeek Harness 可用 [OpenVAIR Desktop](/docs/desktop) 一键接入：粘贴 Key、选择模型、点「接入」，断开时还原原配置。本页是客户端不支持的工具和 SDK 的手动配置。

## 填什么

| 设置项 | 填什么 |
| --- | --- |
| 接口地址 / Base URL / API Host | OpenAI 兼容：`{{origin}}/v1`；Anthropic 和 Gemini：`{{origin}}` |
| API Key | 你的 API Key |
| 模型 | Key 能用的模型名，可在 [Key 查询页](/my-key) 点击复制 |

工具会自动补 `/v1` 时只填 `{{origin}}`；报 404 时试着加上或去掉末尾的 `/v1`。

## CC Switch

在 Claude、Codex 或 Gemini 页面添加自定义供应商，保存后切换到 OpenVAIR。**Codex 页面的请求地址末尾需加 `/v1`。**

```text CC Switch
供应商名称：OpenVAIR
API Key：你的 API Key
请求地址：{{origin}}
```

## 聊天客户端

Cherry Studio、ChatBox、LobeChat、Open WebUI 等：添加「OpenAI 兼容」或「自定义」供应商，按上表填写。模型可手动添加，或点「获取模型列表」获取该 Key 可用的模型。

## OpenAI SDK

```python Python
from openai import OpenAI

client = OpenAI(
    base_url="{{origin}}/v1",
    api_key="你的 API Key",
)

response = client.chat.completions.create(
    model="gpt-4.1-mini",  # 示例，换成你的 Key 能用的模型
    messages=[{"role": "user", "content": "你好"}],
)
print(response.choices[0].message.content)
```

```javascript Node.js
import OpenAI from 'openai'

const client = new OpenAI({
  baseURL: '{{origin}}/v1',
  apiKey: process.env.OPENVAIR_API_KEY,
})

const response = await client.chat.completions.create({
  model: 'gpt-4.1-mini', // 示例，换成你的 Key 能用的模型
  messages: [{ role: 'user', content: '你好' }],
})
console.log(response.choices[0].message.content)
```

## Anthropic SDK

`base_url` **不要**加 `/v1`：

```python Python
from anthropic import Anthropic

client = Anthropic(
    base_url="{{origin}}",
    api_key="你的 API Key",
)

message = client.messages.create(
    model="claude-sonnet-5-5",
    max_tokens=1024,
    messages=[{"role": "user", "content": "你好"}],
)
print(message.content[0].text)
```

## 其他框架

LangChain、LlamaIndex、Vercel AI SDK 等填「自定义 Base URL」参数即可。直接发 HTTP 请求见 [API 参考](/docs/api)。

> [!WARNING]
> 不要将 API Key 写入代码或提交到 Git，请使用环境变量或密钥管理工具。
