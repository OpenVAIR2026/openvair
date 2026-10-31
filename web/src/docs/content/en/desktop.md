OpenVAIR Desktop writes the config for the local agents in [the table below](#connecting-local-agents) from a server address, an API key and a chosen model; for other clients, use [Manual setup](#manually-connecting-other-clients) (手动接入). It doesn't sign in to the console and can't manage keys or channels.

## Download and install

Download from [GitHub Releases](https://github.com/OpenVAIR2026/openvair/releases), or build it by following `client/README.md`.

| System | Install |
| --- | --- |
| macOS | Drag the app from the `.dmg` into **Applications** (应用程序); if blocked, click **Open Anyway** (仍要打开) in **System Settings › Privacy & Security** (系统设置 › 隐私与安全性) |
| Windows | Run the installer; on a SmartScreen prompt, click **More info › Run anyway** (更多信息 › 仍要运行) |
| Linux | `sudo apt install ./<file-name>.deb` or `sudo dnf install ./<file-name>.rpm` |

Installers aren't Apple-notarized or Windows-signed. If macOS says the app is "damaged" (已损坏), run `xattr -cr "/Applications/OpenVAIR Desktop.app"`.

## Add the server and key

1. In **Server address** (服务器地址), enter your OpenVAIR address, e.g. `https://openvair.example.com`; on this machine, `http://localhost:8080`.
2. Paste the key into **OPENVAIR KEY** and click **Add key** (添加 Key); it's saved once the server verifies it.

- Public addresses should use `https://`; a public `http://` address needs your confirmation, and the key and requests then travel in plain text. `localhost`, LAN IPs and `.local` can use `http://` directly. With `http://`, the **Keys** (Key) and **Settings** (设置) pages keep warning you.
- Each client connects to one server.
- Keys are stored in Credential Manager on Windows, the Secret Service on Linux, or a file only the current user can read and write on macOS.

## Managing keys

The **Keys** page shows each key's status, usage, limits and models.

- **+ Add key** (+ 添加 Key): uses the current server.
- **Rename** (重命名), **Set as default** (设为默认): the default key is preselected when connecting an agent.
- **Usage ↗** (用量 ↗): opens the [key lookup page](/docs/my-key).
- **Remove** (移除): deletes only the local copy, without revoking; switch or disconnect agents using it first.

**Invalid** (已失效) means the key was revoked or disabled; ask your admin for a new one.

## Connecting local agents

1. On **Local agents** (本机 Agent), click **+ Connect agent** (+ 接入 Agent), find the agent and click **Connect** (接入).
2. For agents supporting both protocols, choose OpenAI or Anthropic.
3. Choose a key and a model (or type one). Claude Code can also use a **Background task model** (后台任务模型) for titles, context compaction and similar.
4. Review **Changes** (将要修改), click **Write and test** (写入并测试), then restart the agent or start a new session as prompted.

Files are backed up before every change (**Settings › Config backups** (设置 › 配置备份)). The card's **⋯** menu has **Change key** (更换 Key), **Change model** (更换模型), **Test connection** (测试连接) and **Disconnect and restore config** (断开并还原配置). Only these fields change:

| Agent | Files and fields changed |
| --- | --- |
| Claude Code | `ANTHROPIC_BASE_URL`, `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_MODEL` under `env` in `~/.claude/settings.json`; `ANTHROPIC_DEFAULT_HAIKU_MODEL`, `ANTHROPIC_DEFAULT_SONNET_MODEL`, `ANTHROPIC_DEFAULT_OPUS_MODEL` as needed; the key's other models in the `/model` menu (`modelPicker`) |
| Codex | Top-level `model`, `model_provider` and `[model_providers.openvair]` in `~/.codex/config.toml`; the key goes in `experimental_bearer_token`. Sets `model_catalog_json` if the key has models Codex doesn't recognize |
| Gemini CLI | `GOOGLE_GEMINI_BASE_URL`, `GEMINI_API_KEY`, `GEMINI_MODEL` in `~/.gemini/.env`; `security.auth.selectedType` in `~/.gemini/settings.json` |
| Qwen Code | In `~/.qwen/settings.json`: client-added entries under `modelProviders.openai` or `modelProviders.anthropic` (`envKey` = `OPENVAIR_API_KEY`), `env.OPENVAIR_API_KEY`, `security.auth.selectedType`, `model.name` |
| Kimi CLI | `[providers.openvair]`, `[models."openvair/<model-name>"]` and top-level `default_model` in `~/.kimi/config.toml` |
| OpenCode | `provider.vair` and top-level `model` (`openvair/<model-name>`) in `~/.config/opencode/opencode.json` |
| ZCode | The provider with `providerId` `openvair`, `providerOrder` and `defaultModelSelection` in `~/.zcode/v2/provider_config.json` |
| DeepSeek Harness | The block between `# >>> VAIR` and `# <<< VAIR` at the end of `~/.dsh/cordis.patch.yml`, overriding `llm-deepseek` and `agent-default-model`; the key goes in `OPENVAIR_API_KEY` in `~/.dsh/.env` |

- The key is stored in plain text in these files. Don't commit them to Git or share them.
- Disconnecting removes only what the client wrote: your own providers stay and the default model reverts; in DeepSeek Harness your original settings apply again.
- Use **Manual setup** instead when: OpenCode uses `opencode.jsonc` (comments would be lost); ZCode has never been opened (no `provider_config.json`; open it once first) or its data directory was moved.
- These override the client's config: `GEMINI_API_KEY` and `GOOGLE_GEMINI_BASE_URL` env vars for Gemini CLI; `OPENAI_BASE_URL` and `OPENAI_API_KEY` for Kimi CLI on the OpenAI protocol; `.qwen/settings.json` or `opencode.json` in a project directory.
- If ZCode asks you to sign in on first launch, choose **Use API key** (使用 API key), then **Skip for now** (暂时跳过).
- Codex signed in with a ChatGPT account contacts chatgpt.com every turn and times out if unreachable; see [Codex](/docs/codex).

## Model list sync

When your admin changes a key's models, the client updates the `/model` menu and Codex model catalog it wrote, without changing your chosen model. Turn off **Automatically update agent model lists** (自动更新 Agent 的模型列表) on **Settings**, or click **Update now** (立即更新).

## Manually connecting other clients

Cursor, Cherry Studio, Cline and similar: **+ Connect agent** → **Other clients** (其他客户端) → **View connection details** (查看接入信息) to copy each protocol's endpoint, a key (**Copy key** (复制 Key); the full key is never shown) and the key's model names. No files change. See [Other clients and SDKs](/docs/other-clients).

## Long-conversation compression

**Context compression** (上下文压缩) on **Settings** runs the bili proxy locally for Claude Code and Codex, compressing long conversations to cut tokens.

1. Connect the agent on **Local agents**.
2. Install the bili component (needs Node.js 18+).
3. Turn on that agent's switch.

- When on, the agent goes through local bili to OpenVAIR; applies only to **new conversations**. **Open bili UI** (打开 bili 界面) shows compression status.
- Codex: trust the bili startup hook on first start, or the proxy won't start; turn on **Launch at startup** (开机启动) so the client stays in the tray and keeps bili running.

## Switching servers

**Settings › Server** (设置 › 服务器) → **Change server** (更换服务器), then enter the new address.

- **Same server, new IP or domain**: once your keys verify against the new address, all are kept and connected agents switch automatically. If your admin set a new access address, the card offers **Use new address** (改用新地址).
- **A different server**: disconnect all agents, click **Switch to another server (remove all keys)** (换到另一台服务器（移除全部 Key）) (keys aren't revoked), then re-enter.

## Updates and lost devices

- The client doesn't update itself: **Settings › About** (设置 › 关于) → **Check for new version ↗** (查看新版本 ↗), then install over the old version.
- Use one key per device; if a device is lost, ask your admin to revoke its key.
