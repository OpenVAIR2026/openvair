面向管理员，以 Claude Code 为例；其他工具只换第 3 步。需要：控制台登录权限、一把供应商 API Key。尚未部署见[部署与运维](https://github.com/OpenVAIR2026/openvair/blob/main/docs/self-hosting.md)。

## 第 1 步：添加渠道

1. 「上游渠道」→「添加渠道」。
2. 「渠道来源」选「第三方 API Key」（Anthropic 官方需手动填地址），「协议」选「Anthropic 兼容」。
3. 填写「渠道名称」、**Base URL**（如 `https://api.anthropic.com`）、「上游 API Key」和「模型规则」（如 `claude-*`）。只填前缀规则时，「测试模型」须填具体模型名。
4. 「测试连接」通过后点「创建渠道」。

![添加渠道：选择来源和协议，填好地址和 Key 后先测试连接](/docs-img/channel-create.webp)

## 第 2 步：创建 API Key

1. 「API Key」→「创建 API Key」。
2. 填「Key 名称」，「允许模型」填 `claude-*` 或勾选「允许全部模型」，其余默认，点「创建 API Key」。
3. 点「复制 Key」（以 `openvair_` 开头）。

![创建 API Key：名称、可用渠道和模型、限额](/docs-img/key-create.webp)

## 第 3 步：接入 Claude Code

推荐用 [OpenVAIR Desktop](/docs/desktop)：下载安装 → 填服务器地址 `{{origin}}` 并粘贴 Key → 在「本机 Agent」中接入 Claude Code。

也可手动设置环境变量：

```bash 终端
export ANTHROPIC_BASE_URL="{{origin}}"
export ANTHROPIC_AUTH_TOKEN="你的 API Key"
claude
```

Key 的「更多操作」（⋯）→「接入说明」→「复制配置」可得到已填好地址和 Key 的配置。永久生效见 [Claude Code](/docs/claude-code#永久生效)。

![「接入说明」面板：选好工具，复制的配置里已经填好地址和 Key](/docs-img/key-connect.webp)

## 第 4 步：确认请求记录

「使用分析」→「调用明细」中可看到这次请求。

![「调用明细」里的每一行是一次请求](/docs-img/usage-requests.webp)

接下来：[备用渠道](/docs/routing)、[限额](/docs/limits)、[OpenVAIR Desktop](/docs/desktop)。
