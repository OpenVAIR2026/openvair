# OpenVAIR Desktop

桌面客户端：填写 OpenVAIR 服务器地址和 API Key 后，把 Key 配置到本机的 Codex、Claude Code、Gemini CLI、Qwen Code、Kimi CLI、OpenCode、ZCode 和 DeepSeek Harness。一个客户端对应一个服务器，可以保存多把 Key，每个 Agent 分别选择 Key 和模型。其他客户端通过「手动接入」面板复制地址、Key 和模型。客户端不登录控制台帐号，也不创建或撤销 Key。设计见 [`docs/design/local-client-design.md`](../docs/design/local-client-design.md)。

<img src="../web/public/docs-img/desktop-agents.webp" width="860" alt="OpenVAIR Desktop 的本机 Agent 页">

## 结构

- `core/`：Rust 库，负责 Key 摘要接口（`/api/key/summary`）、Key 存储（Windows 凭据管理器、Linux Secret Service；macOS 和调试构建用 `0600` 文件，因为未签名应用每次更新都会重新弹钥匙串授权）、Agent 配置读写与备份、连通性测试；不依赖界面。每个 Agent 是 `core/src/agents/` 下实现 `Adapter` trait 的一个模块，新增 Agent 时在 `AgentId` 和 `ALL` 里登记。
- `src-tauri/`：Tauri 2 应用，把 `core` 包装成界面可调用的命令。密钥只在这一侧出现，界面只看到打码后的值。
- `src/`：React 界面。

## 开发

```bash
npm --prefix client ci
npm --prefix client run tauri dev
```

只调界面时可以在浏览器中运行 `npm --prefix client run dev`：不在 Tauri 中时会自动使用 `src/lib/mock.ts` 的示例数据（仅开发构建）。`?state=welcome` 显示首次启动页，`?state=lan` 模拟局域网 HTTP 服务器，`?agents=many` 接入多个 Agent 以显示滚筒。

和已安装的正式版同时运行时，用单独的数据目录和应用标识，避免两边共用 `state.json`：

```bash
cd client
OPENVAIR_DATA_DIR="$HOME/Library/Application Support/OpenVAIR Dev" npm run tauri dev -- --config '{"identifier":"com.openvair.desktop.dev","productName":"OpenVAIR Dev"}'
```

两边仍然改写同一份 Agent 配置文件。

连本地服务端时，服务器地址填写 `http://localhost:8080`。HTTP 地址都可以连接：本机、局域网和 Tailscale 地址只显示未加密提示；公网地址要先确认 Key 会明文传输。`?state=public-http` 模拟公网 HTTP 服务器。

## 验证

```bash
cd client
cargo fmt --all
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
npm run build
```

## 发布

`npm run tauri build` 在本机生成当前平台的安装包。开源版不带自动更新，「关于」页链接到 [GitHub Releases](https://github.com/OpenVAIR2026/openvair/releases) 查看新版本。
