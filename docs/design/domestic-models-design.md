# 国产模型接入

## 目标

让 DeepSeek、Kimi、GLM、Qwen、MiniMax、豆包等国产模型能用 OpenVAIR 现有的三种协议（`openai`、`anthropic`、`gemini`）接入，做到：

1. **能用**：各厂商的官方 base_url 都能正确拼出上游地址。
2. **算对**：缓存命中、用量和费用记录准确。
3. **好配**：添加渠道时选择厂商，协议和地址自动填写。

## 不做的事

- **不新增 `protocol`。** 国产厂商都提供 OpenAI 兼容端点，多数还提供 Anthropic 兼容端点。路由仍然要求请求协议和渠道协议一致。
- **不做协议互转。** 例如让 Gemini CLI 调用国产模型，这和「透传、不改语义」的定位冲突。
- **不改写思考参数。** `thinking`、`enable_thinking`、`reasoning_effort` 原样透传。
- 暂不支持多币种定价、阶梯价和国产模型分词器。

## base_url 版本段拼接

部分厂商的 base_url 以 `/v1` 以外的版本段结尾，例如智谱 `https://open.bigmodel.cn/api/paas/v4`、豆包方舟 `https://ark.cn-beijing.volces.com/api/v3`。如果直接拼上请求路径，会得到 `.../v4/v1/chat/completions` 这样的 404 地址。

`join_base_url`（`src/gateway.rs`）的规则：

```text
base 最后一段是 v<数字> 且 path 以 /v1/ 开头       →  base + path 去掉开头的 /v1
base 以 /v1beta 结尾 且 path 以 /v1beta/ 开头      →  base + path 去掉开头的 /v1beta
其他                                               →  base + path
```

base 末尾的 `/` 会先去掉。网关转发、获取上游模型、渠道测试和可用性监控都经过这个函数。

## 用量与缓存字段

`parse_usage` 读取缓存命中数时依次查看 `prompt_tokens_details.cached_tokens`、`input_tokens_details.cached_tokens`、`cache_read_input_tokens`、`cachedContentTokenCount`，最后兜底读取 DeepSeek 旧版顶层字段 `prompt_cache_hit_tokens` 和 Kimi 旧版顶层字段 `cached_tokens`。

- 标准字段优先，兜底字段只在末尾，不影响已有厂商。
- `input` 的语义不变：OpenAI 兼容端点的 `prompt_tokens` 已经包含缓存命中部分，DeepSeek 的 `prompt_cache_miss_tokens` 不参与计算，不会重复累加。
- 读不到缓存字段时 `cached` 为空，缓存命中部分按全价输入计费，`cache_usage_reported` 记为未上报。
- 流式请求沿用 `parse_stream_usage` 的取最大值合并逻辑。网关为 Chat Completions 自动注入 `stream_options.include_usage`，让流式响应的最后一个 chunk 带上 `usage`。

DeepSeek 和 Kimi 的格式已按官方文档写了单元测试。Qwen、GLM、豆包、MiniMax 以及各家的 Anthropic 端点预计使用标准字段，尚未用真实响应验证。

## 厂商预设

预设是 `src/providers.rs` 中的静态配置，不建数据库表。厂商端点不常变，随版本更新即可。

```rust
pub struct Provider {
    pub id: &'static str,
    pub label: &'static str,
    pub docs_url: &'static str,
    pub endpoints: &'static [Endpoint],
}

pub struct Endpoint {
    pub protocol: &'static str,
    pub base_url: &'static str,
}
```

| id | 厂商 | OpenAI 端点 | Anthropic 端点 |
| --- | --- | --- | --- |
| `deepseek` | DeepSeek | `https://api.deepseek.com` | `https://api.deepseek.com/anthropic` |
| `moonshot` | Kimi（月之暗面） | `https://api.moonshot.cn/v1` | `https://api.moonshot.cn/anthropic` |
| `zhipu` | 智谱 GLM | `https://open.bigmodel.cn/api/paas/v4` | `https://open.bigmodel.cn/api/anthropic` |
| `dashscope` | 通义千问（阿里云百炼） | `https://dashscope.aliyuncs.com/compatible-mode/v1` | `https://dashscope.aliyuncs.com/apps/anthropic` |
| `minimax` | MiniMax | `https://api.minimaxi.com/v1` | `https://api.minimaxi.com/anthropic` |
| `volcengine` | 豆包（火山方舟） | `https://ark.cn-beijing.volces.com/api/v3` | `https://ark.cn-beijing.volces.com/api/compatible` |

预设只收录国内站。国际站的 Key 请改用国际站地址或自定义渠道。预设只填写默认值，鉴权、路由和计费仍按渠道协议处理。

### 数据库

迁移 `0072_channel_provider.sql`：

```sql
ALTER TABLE channels ADD COLUMN provider TEXT NULL;
```

- 可为空。早于预设的渠道和「自定义」渠道都是 `NULL`。
- 不加 CHECK 约束。预设 ID 由代码校验，增删厂商不需要新迁移。

### 后端

- `GET /api/providers`：返回预设列表，需要登录，不含帐号数据。
- 创建渠道时接受可选的 `provider`。传入时校验 ID 存在，且该厂商有对应协议的端点。base_url 以用户提交的为准。
- 渠道列表和详情返回 `provider`。

### 控制台

渠道编辑器（`web/src/components/channel-editor.tsx`）：

1. 新建渠道时先选「厂商」，或选「第三方 API Key」等自定义来源。
2. 选了厂商后，协议只显示该厂商支持的协议，并自动填入国内站 base_url，用户仍可修改。
3. 编辑已有渠道时显示「创建自厂商预设：…」，不能更改厂商；要换厂商就新建渠道。

## 验证

```bash
cargo fmt --all
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
npm --prefix web ci
npm --prefix web run build
```

单元测试覆盖 `join_base_url` 的 `/api/paas/v4`、`/api/v3`、末尾带 `/`、`/anthropic`、`/v1beta` 等情况，预设的唯一性和格式，以及 DeepSeek、Kimi 的用量字段。用真实 Key 验证厂商时，请使用临时渠道，不要把 Key 写进仓库或测试代码。
