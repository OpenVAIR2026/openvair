<div align="center">

<img src="client/app-icon.svg" width="88" alt="OpenVAIR">

# OpenVAIR

**一条命令搭好，粘贴一把 Key 就能用。**

团队自部署的 AI 网关，让 Claude Code、Codex、Gemini CLI 等编程 Agent 使用你自己的模型渠道。

[![License: AGPL-3.0](https://img.shields.io/badge/license-AGPL--3.0-0b171c?style=flat-square)](LICENSE)
[![Desktop: Apache-2.0](https://img.shields.io/badge/desktop-Apache--2.0-173238?style=flat-square)](client/LICENSE)
![Rust](https://img.shields.io/badge/server-Rust-78d9d0?style=flat-square&logo=rust&logoColor=white)
![Docker](https://img.shields.io/badge/docker-amd64%20%7C%20arm64-78d9d0?style=flat-square&logo=docker&logoColor=white)

**中文** · [English](README.en.md)

[快速开始](#快速开始) · [桌面客户端](client/README.md) · [部署与运维](docs/self-hosting.md) · [和其他项目对比](docs/comparison.md)

<br>

<img src="web/public/docs-img/desktop-agents.webp" width="900" alt="OpenVAIR Desktop 的本机 Agent 页：每个 Agent 用哪把 Key、哪个模型、今日用了多少">

</div>

## 特点

- **一条命令安装**：预构建镜像，密钥自动生成，浏览器向导设置管理员。
- **概念少**：单租户、一个管理员，没有注册、成员、角色和充值。
- **默认安全**：上游 Key 加密保存，防 SSRF，Secure Cookie 和安全响应头默认开启。
- **粘贴 Key 就能用**：OpenVAIR Desktop 一键配置 Codex、Claude Code、Gemini CLI 等 Agent，自动测试，断开时恢复原配置。
- **按人发 Key**：每把 Key 单独限定渠道、模型、额度、速率和并发，随时停用或撤销。
- **任意渠道**：官方 API、中转站或内网服务，支持 OpenAI、Anthropic、Gemini 及兼容协议，对外同时提供三种协议。
- **自动切换**：报错或限流时按优先级切换，出错渠道短暂冷却。
- **渠道监控**：定时检查可用性和质量，可自动降级或剔除。
- **好排查**：请求详情显示经过的渠道、失败原因和耗时分布。
- **用量清楚**：按 Key、模型和渠道统计请求、Token 和费用，不存正文；持有人在客户端自查，无需帐号。

<div align="center">
<img src="web/public/docs-img/desktop-keys.webp" width="900" alt="OpenVAIR Desktop 的 Key 页：每把 Key 的今日费用、额度、限速和可用模型">
</div>

## 快速开始

**① 安装服务端**（需要 Docker 和 Docker Compose v2）

```bash
curl -fsSL https://raw.githubusercontent.com/OpenVAIR2026/openvair/main/install.sh | sh
```

脚本安装到 `~/openvair`，启动后打印访问地址和初始化令牌。升级时再运行一次。

**② 添加渠道和 Key**

打开 <http://localhost:8080/setup>（或脚本打印的地址）设置管理员，添加上游渠道，为每位同事创建 API Key。

**③ 同事接入**

在 [OpenVAIR Desktop](https://github.com/OpenVAIR2026/openvair/releases) 填入服务器地址和 Key，选择 Agent，点「接入」。

> [!IMPORTANT]
> **请备份 `MASTER_KEY`**，丢失后渠道 Key 无法解密：
> `cd ~/openvair && docker compose exec openvair cat /run/openvair/master_key`

<details>
<summary><b>从源码构建</b></summary>

```bash
git clone https://github.com/OpenVAIR2026/openvair.git
cd openvair
OPENVAIR_IMAGE=openvair:local docker compose up -d --build
```

</details>

<details>
<summary><b>局域网和公网部署</b></summary>

局域网用 `http://服务器IP:8080`，公网绑定域名并经 HTTPS 反向代理，见[部署与运维](docs/self-hosting.md)。

</details>

## 不用客户端也能调

```bash
curl http://localhost:8080/v1/chat/completions \
  -H 'Authorization: Bearer openvair_...' \
  -H 'Content-Type: application/json' \
  -d '{"model":"your-model","messages":[{"role":"user","content":"hello"}]}'
```

Key 可放在 `Authorization: Bearer`、`x-api-key`（Anthropic）或 `x-goog-api-key`（Gemini）请求头，按此顺序读取。

## 可选配置

均有默认值，写入部署目录的 `.env` 后执行 `docker compose up -d`。

| 变量 | 说明 |
| --- | --- |
| `OPENVAIR_BIND` / `OPENVAIR_PORT` | 监听地址和端口，默认 `127.0.0.1:8080` |
| `COOKIE_SECURE` | 默认 `true`；仅用 HTTP 访问非本机地址时设为 `false` |
| `TRUSTED_PROXIES` | 可信代理网段，默认本机和私有网段 |
| `INITIAL_ADMIN_EMAIL` / `INITIAL_ADMIN_PASSWORD` | 首次启动时创建管理员 |
| `ALLOW_PRIVATE_UPSTREAMS` | 渠道能否指向内网，默认 `true`；不信任的人能访问控制台时设为 `false` |
| `OPENVAIR_IMAGE` | 默认 `ghcr.io/openvair2026/openvair:latest` |

## 适合谁

✅ 小团队、公司或实验室内部共用模型渠道。

❌ 多实例高可用、SSO、多成员角色、对外转售或充值计费，见[对比](docs/comparison.md)。

## 参与开发

后端 Rust，控制台 React + Vite，桌面客户端 Tauri 2，见 [CONTRIBUTING.md](CONTRIBUTING.md)。

## 许可证

服务端和控制台为 [GNU AGPL-3.0](LICENSE)，桌面客户端（`client/`）为 [Apache-2.0](client/LICENSE)，`src/ctok/` 移植自 [sanderland/ctok](https://github.com/sanderland/ctok)（MIT）。见 [NOTICE](NOTICE)。

安全问题请按 [SECURITY.md](SECURITY.md) 私下报告。
