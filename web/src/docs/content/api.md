请求和响应格式与各家官方 API 一致：地址换成 `{{origin}}`，Key 换成 OpenVAIR 的 API Key。OpenVAIR 只按需改写模型名（见[模型映射](/docs/models#模型映射)），不转换协议（见[协议必须一致](/docs/concepts#协议必须一致)）。

## 鉴权

按以下顺序读取 API Key：

| 请求头 | 常用于 |
| --- | --- |
| `Authorization: Bearer 你的 API Key` | OpenAI SDK、大多数客户端 |
| `x-api-key: 你的 API Key` | Anthropic SDK |
| `x-goog-api-key: 你的 API Key` | Gemini SDK |

API Key 不会转发给上游，上游使用渠道自己的 Key。

## 支持的接口

| 接口 | 协议 | 模型名位置 |
| --- | --- | --- |
| `POST /v1/chat/completions` | OpenAI Chat Completions | 请求体 `model` |
| `POST /v1/responses` | OpenAI Responses | 请求体 `model` |
| `POST /v1/messages` | Anthropic Messages | 请求体 `model` |
| `POST /v1beta/models/{model}:generateContent` | Gemini | 路径 |
| `POST /v1beta/models/{model}:streamGenerateContent` | Gemini 流式 | 路径 |
| `GET /v1/models`、`GET /v1beta/models` | 模型列表 | 只返回该 Key 可用的模型 |

- 流式输出原样转发；请求体最大 50 MB。
- 其他接口返回 404 `not_found`，如 `/v1/embeddings`、`/v1/messages/count_tokens`、图像和语音接口。
- 除模型列表外都必须用 `POST`，否则返回 400；模型名不能带 `*`。
- 两个模型列表接口都返回 OpenAI 格式 `{"object": "list", "data": [{"id": …}]}`，Gemini SDK 的列出模型功能无法直接解析。查询模型列表不调用上游、不产生费用，可用来检查 Key。

## 示例

先设置环境变量 `OPENVAIR_API_KEY`，模型名换成你的 Key 可用的模型。

```bash OpenAI Chat Completions
curl {{origin}}/v1/chat/completions \
  -H "Authorization: Bearer $OPENVAIR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4.1-mini",
    "messages": [{"role": "user", "content": "你好"}]
  }'
```

```bash Anthropic Messages
curl {{origin}}/v1/messages \
  -H "x-api-key: $OPENVAIR_API_KEY" \
  -H "anthropic-version: 2023-06-01" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "claude-sonnet-5-5",
    "max_tokens": 1024,
    "messages": [{"role": "user", "content": "你好"}]
  }'
```

```bash Gemini
curl "{{origin}}/v1beta/models/gemini-2.5-flash:generateContent" \
  -H "x-goog-api-key: $OPENVAIR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"contents": [{"parts": [{"text": "你好"}]}]}'
```

```bash 查询可用模型
curl {{origin}}/v1/models \
  -H "Authorization: Bearer $OPENVAIR_API_KEY"
```

## 错误响应

```json 错误格式
{
  "error": {
    "code": "model_not_allowed",
    "message": "此 Key 无权使用模型 gpt-4o。在 {{origin}}/my-key 查看此 Key 的状态与额度"
  }
}
```

除 `invalid_api_key` 外，`message` 末尾都附有 Key 查询页地址。全部错误码和处理方法见[问题排查](/docs/troubleshooting#按错误码查找)。上游返回错误且没有其他渠道可切换时，原样返回上游的状态码和响应。
