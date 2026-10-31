For admins, using Claude Code as the example; for other tools, only step 3 changes. You need console access and a provider API key. Not deployed yet? See [Deployment and operations](https://github.com/OpenVAIR2026/openvair/blob/main/docs/self-hosting.en.md).

## Step 1: Add a channel

1. **Upstream channels** (上游渠道) → **Add channel** (添加渠道).
2. **Channel source** (渠道来源): **Third-party API key** (第三方 API Key); Anthropic's official API needs its URL entered manually. **Protocol** (协议): **Anthropic-compatible** (Anthropic 兼容).
3. Fill in **Channel name** (渠道名称), **Base URL** (e.g. `https://api.anthropic.com`), **Upstream API key** (上游 API Key) and **Model rules** (模型规则, e.g. `claude-*`). With only prefix rules, enter a concrete model in **Test model** (测试模型).
4. Once **Test connection** (测试连接) passes, click **Create channel** (创建渠道).

![Adding a channel: choose the source and protocol, fill in the URL and key, then test the connection first](/docs-img/channel-create.webp)

## Step 2: Create an API key

1. **API Key** → **Create API key** (创建 API Key).
2. Fill in **Key name** (Key 名称), set **Allowed models** (允许模型) to `claude-*` or check **Allow all models** (允许全部模型), keep other defaults, and click **Create API key**.
3. Click **Copy key** (复制 Key); it starts with `openvair_`.

![Creating an API key: name, available channels and models, limits](/docs-img/key-create.webp)

## Step 3: Connect Claude Code

We recommend [OpenVAIR Desktop](/docs/desktop): download and install → enter the server address `{{origin}}` and paste the key → connect Claude Code under **Local agents** (本机 Agent).

Or set the environment variables manually:

```bash Terminal
export ANTHROPIC_BASE_URL="{{origin}}"
export ANTHROPIC_AUTH_TOKEN="your API key"
claude
```

The key's **More actions** (更多操作, ⋯) → **Setup instructions** (接入说明) → **Copy config** (复制配置) gives a config with the URL and full key filled in. To make it permanent, see [Claude Code](/docs/claude-code#make-it-permanent).

![The Setup instructions panel: pick a tool, and the copied config already includes the URL and key](/docs-img/key-connect.webp)

## Step 4: Check the request log

The request appears in **Usage analytics** (使用分析) → **Request details** (调用明细).

![Each row in Request details is one request](/docs-img/usage-requests.webp)

Next: [fallback channels](/docs/routing), [limits](/docs/limits), [OpenVAIR Desktop](/docs/desktop).
