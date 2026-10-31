# OpenVAIR Desktop 设计

OpenVAIR Desktop（`client/`）是桌面客户端，把 OpenVAIR 的 API Key 写进本机 Agent 的配置文件，不必手动复制 Base URL、Key 和模型名。它只用 API Key，不登录控制台帐号。

不做的事：

- 不管理渠道、上游 Key、路由和监控，这些在网页控制台里完成；
- 不创建、停用或撤销 Key，客户端只使用用户粘贴进来的 Key；
- 不做本地转发或协议转换，Agent 直接请求网关的 `/v1`、`/v1beta`。开启 bili 长对话压缩时例外，见下文。

## 形态

基于 Tauri 2，支持 macOS、Windows 和 Linux。

- `client/core`：Rust 库，负责 Key 摘要接口、密钥存储、本地状态、Agent 配置读写与备份、连通性测试，不依赖界面。
- `client/src-tauri`：把 `core` 包装成 Tauri 命令。完整的 Key 只在这一侧出现，界面只看到打码后的值。
- `client/src`：React 界面，页面有 Agent、Key 和设置。

## 服务器和 Key

一台电脑上的客户端只对应一个 OpenVAIR 服务器，可以保存多把 Key。

- **首次启动**：填写服务器地址和一把 API Key。客户端调用 `GET /api/key/summary` 校验 Key，校验通过后同时记下服务器和 Key。地址可以省略协议，默认补成 `https://`。`http://` 地址都可以使用：`localhost`、`*.localhost`、`*.local`、`*.lan`、`*.internal`、`*.home.arpa`、不带点的主机名、回环地址、私有网段和 Tailscale 使用的 `100.64.0.0/10` 只在界面上提示未加密；其他地址视为公网，添加第一把 Key 时要先确认 Key 和对话内容会明文传输，之后 Key 列表和设置页一直显示明文传输的提示。
- **添加 Key**：之后添加的 Key 都发往同一个服务器，被拒绝时提示这把 Key 不属于当前服务器或已失效。
- **管理 Key**：可以给 Key 起本地名称、设为默认 Key 或移除。有 Agent 正在使用的 Key 不能移除，需要先让这些 Agent 改用其他 Key 或断开。移除只删除本机保存的副本，不会撤销服务器上的 Key。
- **Key 详情**：显示 `/api/key/summary` 返回的状态、额度进度和各协议可用的模型，也可以复制 Key，或在浏览器中打开服务器上的用量页 `/my-key/usage`。
- **更换服务器**：在设置页操作。新地址上每把已保存的 Key 都通过 `/api/key/summary` 验证且 ID 不变时，视为同一台服务器换了地址：保留全部 Key，已接入且配置文件仍是本机写入的 Agent 改用新地址（清除撤销记录，避免撤销时写回旧地址），其余跳过。否则可以选择移除全部 Key 换到另一台服务器，仍有 Agent 接入时拒绝。Key 摘要里的 `server_url` 是管理员设置的访问地址，与当前地址不同时设置页会提示。

服务器端的 Key 自助接口见 [Key 持有者自助](key-holder-design.md)。

## 接入 Agent

支持 Codex、Claude Code、Gemini CLI、Qwen Code、Kimi CLI、OpenCode、ZCode 和 DeepSeek Harness。每个 Agent 是 `client/core/src/agents/` 下实现 `Adapter` trait 的一个模块，在 `AgentId` 和 `ALL` 中登记。

| Agent | 协议 | 写入的文件（相对用户目录） |
| --- | --- | --- |
| Codex | OpenAI | `.codex/config.toml`、`.codex/openvair-models.json` |
| Claude Code | Anthropic | `.claude/settings.json` |
| Gemini CLI | Gemini | `.gemini/.env`、`.gemini/settings.json` |
| Qwen Code | OpenAI / Anthropic | `.qwen/settings.json` |
| Kimi CLI | OpenAI / Anthropic | `.kimi/config.toml` |
| OpenCode | OpenAI / Anthropic | `.config/opencode/opencode.json` |
| ZCode | OpenAI / Anthropic | `.zcode/` 下的设置文件 |
| DeepSeek Harness | Anthropic | `.dsh/cordis.patch.yml`、`.dsh/.env` |

支持多种协议的 Agent 默认用表中的第一种，可以在接入时切换。

接入流程：

1. 为 Agent 选择一把 Key（默认是默认 Key）和这把 Key 在该协议下可用的模型。Claude Code 还可以选择后台模型。
2. 预览将要修改的文件和字段，Key 打码显示。
3. 写入前重新调用 `/api/key/summary` 确认 Key 仍然有效且没有被停用，然后把要修改的文件备份到数据目录的 `backups/`，只改 OpenVAIR 相关的字段。文件通过临时文件加重命名写入，权限为 `0600`。
4. 写入后自动做一次连通性测试：用这把 Key 请求网关的模型列表（OpenAI、Anthropic 用 `/v1/models`，Gemini 用 `/v1beta/models`），显示结果和延迟。

接入后可以：

- **测试**：再做一次连通性测试；
- **更换 Key 或模型**：重新走接入流程，保留第一次接入前用户自己的值；
- **撤销上次修改**：用上次写入前的备份恢复文件，并恢复上一次的接入记录；
- **断开**：删除 OpenVAIR 写入的字段，恢复第一次接入前的值；
- **同步模型**：默认开启。Key 可用的模型变化后，后台更新已接入 Agent 配置里的模型列表，并在卡片上显示增减了哪些模型；用户自己改过地址的 Agent 不会被改动。也可以在设置页关闭同步，手动触发。

其他客户端可以用「手动接入」面板复制各协议的 Base URL、Key 和模型。

## 本地数据

- 数据目录为系统应用数据目录下的 `OpenVAIR`，可以用环境变量 `OPENVAIR_DATA_DIR` 改到别处。
- `state.json`：服务器、已保存 Key 的 id、名称和前缀、默认 Key、各 Agent 的接入记录（Key、模型、协议、被替换的原值、上次备份、上次测试结果）和模型同步开关。不保存 Key 本身。
- Key 本身每把单独保存：Windows 用凭据管理器，Linux 用 Secret Service，服务名 `OpenVAIR Desktop`，帐户名 `key <id>`。macOS 和调试构建、以及凭据存储不可用时，保存为数据目录下只有当前用户可读的 `0600` 文件。macOS 不用钥匙串，是因为未签名的应用每次更新代码签名都会变，每次都要重新授权；而 Agent 的配置文件本来就以明文保存 Key。

## 长对话压缩（bili）

bili 是 `billion-context` npm 包，通过本机代理压缩长对话。开启后，Agent 的地址改为经本机代理转发，例如 `http://127.0.0.1:18787/bili/https://openvair.example.com/v1`；代理没有运行时，Agent 连不上网关。

- 设置页的 bili 卡片负责安装、开启和关闭，固定使用经过验证的 bili 版本。
- Claude Code 使用 bili 自带的 SessionStart 钩子启动代理并固定地址。关闭时 bili 会删除地址，客户端再把 OpenVAIR 的地址写回去。
- Codex 没有这样的钩子，由客户端守护代理：关闭窗口后应用留在系统托盘，大约每 30 秒检查一次 `/__bili/health`，代理停止时以独立进程重新启动，所以应用退出或更新不会让代理停掉。端口被其他程序占用时报端口冲突，不再重试。
- 开启前和守护检查时，会按 Agent 实际使用的系统代理或环境变量测试一次本机代理地址能否连通。如果代理软件把 `127.0.0.1` 转发到了远程节点，就不开启，并提示让 `127.0.0.1` 直连。
- 设置页可以开启开机启动（默认关闭），开机后只进入托盘。

## 发布

开源版没有自动更新。「关于」中的链接指向 <https://github.com/OpenVAIR2026/openvair/releases>，新版本在那里发布。安装包用 `npm run tauri build` 在本机构建。

## 安全

- 写进 Agent 配置文件的 Key 只能是明文，因为 Agent 要读取它。客户端把这些文件设为 `0600`，建议每台设备、每个 Agent 使用一把单独的 Key，泄露时可以单独停用或撤销。
- 服务器拒绝某把 Key（已撤销或不存在）时，接入和测试会显示原因，提示换一把 Key。
- 客户端只能打开服务器上以 `/` 开头的页面和 HTTPS 外部链接；bili 面板地址取自 Agent 配置，并且只能是回环地址。

## 测试

```bash
cd client
cargo fmt --all
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
npm run build
```

`client/core` 的单元测试在临时 HOME 中覆盖各 Agent 的写入、与已有配置的合并、备份与恢复、断开，以及服务器地址校验和 bili 的代理检测。
