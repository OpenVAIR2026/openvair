OpenVAIR Desktop 用服务器地址、API Key 和所选模型，自动写好[下表](#接入本机-agent)中本机 Agent 的配置，其他客户端用[手动接入](#手动接入其他客户端)。它不登录控制台，不能管理 Key 或渠道。

## 下载和安装

从 [GitHub Releases](https://github.com/OpenVAIR2026/openvair/releases) 下载，或按 `client/README.md` 自行构建。

| 系统 | 安装 |
| --- | --- |
| macOS | 把 `.dmg` 中的应用拖进「应用程序」；被拦截时在「系统设置 › 隐私与安全性」点「仍要打开」 |
| Windows | 运行安装程序；SmartScreen 提示时点「更多信息 › 仍要运行」 |
| Linux | `sudo apt install ./文件名.deb` 或 `sudo dnf install ./文件名.rpm` |

安装包未经 Apple 公证和 Windows 签名。macOS 提示「已损坏」时运行 `xattr -cr "/Applications/OpenVAIR Desktop.app"`。

## 添加服务器和 Key

1. 「服务器地址」填 OpenVAIR 地址，如 `https://openvair.example.com`；本机填 `http://localhost:8080`。
2. 「OPENVAIR KEY」粘贴 Key，点「添加 Key」，验证通过后保存。

- 公网地址应使用 `https://`；公网 `http://` 需要确认后才能添加，Key 和请求将明文传输。`localhost`、局域网 IP 和 `.local` 可直接用 `http://`。使用 `http://` 时「Key」和「设置」页会持续提示。
- 一个客户端只连一台服务器。
- Key 保存在 Windows 凭据管理器、Linux Secret Service，或 macOS 中仅当前用户可读写的文件。

## 管理 Key

「Key」页显示每把 Key 的状态、用量、限额和可用模型。

- **+ 添加 Key**：沿用当前服务器。
- **重命名**、**设为默认**：接入 Agent 时默认选中默认 Key。
- **用量 ↗**：打开 [Key 查询页](/docs/my-key)。
- **移除**：只删本机副本，不撤销 Key；仍有 Agent 使用时需先更换 Key 或断开。

「已失效」表示 Key 已被撤销或停用，请向管理员申请新 Key。

## 接入本机 Agent

1. 「本机 Agent」页点「+ 接入 Agent」，找到 Agent 点「接入」。
2. 支持两种协议的 Agent 先选 OpenAI 或 Anthropic。
3. 选择 Key 和模型（可手动输入）。Claude Code 还可选「后台任务模型」（生成标题、压缩上下文等）。
4. 核对「将要修改」，点「写入并测试」，再按提示重启 Agent 或新开会话。

修改前自动备份（「设置 › 配置备份」）。卡片「⋯」菜单有「更换 Key」「更换模型」「测试连接」和「断开并还原配置」。只修改下表字段：

| Agent | 修改的文件和字段 |
| --- | --- |
| Claude Code | `~/.claude/settings.json` 中 `env` 的 `ANTHROPIC_BASE_URL`、`ANTHROPIC_AUTH_TOKEN`、`ANTHROPIC_MODEL`；按需设置 `ANTHROPIC_DEFAULT_HAIKU_MODEL`、`ANTHROPIC_DEFAULT_SONNET_MODEL`、`ANTHROPIC_DEFAULT_OPUS_MODEL`，并把其他模型加到 `/model` 菜单（`modelPicker`） |
| Codex | `~/.codex/config.toml` 顶层的 `model`、`model_provider` 和 `[model_providers.openvair]`；Key 写在 `experimental_bearer_token`。有 Codex 不认识的模型时设置 `model_catalog_json` |
| Gemini CLI | `~/.gemini/.env` 中的 `GOOGLE_GEMINI_BASE_URL`、`GEMINI_API_KEY`、`GEMINI_MODEL`，以及 `~/.gemini/settings.json` 中的 `security.auth.selectedType` |
| Qwen Code | `~/.qwen/settings.json` 中 `modelProviders.openai` 或 `modelProviders.anthropic` 下客户端添加的条目（`envKey` 为 `OPENVAIR_API_KEY`）、`env.OPENVAIR_API_KEY`、`security.auth.selectedType` 和 `model.name` |
| Kimi CLI | `~/.kimi/config.toml` 中的 `[providers.openvair]`、`[models."openvair/模型名"]` 和顶层 `default_model` |
| OpenCode | `~/.config/opencode/opencode.json` 中的 `provider.vair` 和顶层 `model`（值为 `openvair/模型名`） |
| ZCode | `~/.zcode/v2/provider_config.json` 中 `providerId` 为 `openvair` 的供应商、`providerOrder` 和 `defaultModelSelection` |
| DeepSeek Harness | `~/.dsh/cordis.patch.yml` 末尾 `# >>> VAIR` 与 `# <<< VAIR` 之间的段落，覆盖 `llm-deepseek` 和 `agent-default-model`；Key 写在 `~/.dsh/.env` 的 `OPENVAIR_API_KEY` |

- Key 以明文写入这些文件，不要提交到 Git 或发给他人。
- 断开只移除客户端写入的部分：自行添加的供应商保留，默认模型恢复为接入前的值；DeepSeek Harness 去掉该段后原设置重新生效。
- 不会改写、需「手动接入」的情况：OpenCode 配置为 `opencode.jsonc`（改写会丢注释）；ZCode 从未打开过（无 `provider_config.json`，可先打开一次）或数据目录已移走。
- 以下设置优先于客户端写入的配置：Gemini CLI 的环境变量 `GEMINI_API_KEY`、`GOOGLE_GEMINI_BASE_URL`；Kimi CLI 用 OpenAI 协议时的 `OPENAI_BASE_URL`、`OPENAI_API_KEY`；项目目录中的 `.qwen/settings.json`、`opencode.json`。
- ZCode 首次打开要求登录时，选「使用 API key」再点「暂时跳过」。
- Codex 用 ChatGPT 帐号登录时每轮都访问 chatgpt.com，访问不了会超时，见 [Codex](/docs/codex)。

## 模型列表同步

管理员调整 Key 的可用模型后，客户端自动更新已写入的 `/model` 菜单和 Codex 模型目录，不改所选模型。可在「设置」关闭「自动更新 Agent 的模型列表」，或点「立即更新」。

## 手动接入其他客户端

Cursor、Cherry Studio、Cline 等：「+ 接入 Agent」→「其他客户端」→「查看接入信息」，可复制各协议接口地址、Key（「复制 Key」，界面不显示完整 Key）和可用模型名，不修改任何文件。填法见[其他客户端与 SDK](/docs/other-clients)。

## 长对话压缩

「设置」页的「上下文压缩」为 Claude Code 和 Codex 在本机运行 bili 代理，压缩长对话以减少 Token。

1. 在「本机 Agent」页接入对应 Agent。
2. 安装 bili 组件（需要 Node.js 18 或更新版本）。
3. 打开该 Agent 的开关。

- 开启后 Agent 经本机 bili 转发到 OpenVAIR，只对**新开的对话**生效；「打开 bili 界面」查看压缩情况。
- Codex：首次启动须信任 bili 启动钩子，否则代理不会启动；建议打开「开机启动」，让客户端驻留托盘保持 bili 运行。

## 更换服务器

「设置 › 服务器」→「更换服务器」，填写新地址。

- **同一服务器换了 IP 或域名**：本机 Key 在新地址验证通过后全部保留，已接入的 Agent 自动改用新地址。管理员设置了新访问地址时，卡片会提示「改用新地址」。
- **换到另一台服务器**：先断开所有 Agent，再点「换到另一台服务器（移除全部 Key）」（不会撤销），重新填写。

## 更新和设备丢失

- 客户端不自动更新：「设置 › 关于」→「查看新版本 ↗」，下载覆盖安装。
- 建议每台设备一把 Key；设备丢失时请管理员撤销该 Key。
