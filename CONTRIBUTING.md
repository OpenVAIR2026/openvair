**中文** · [English](CONTRIBUTING.en.md)

# 参与贡献

感谢你愿意改进 OpenVAIR。

## 提交问题

- Bug：写明 OpenVAIR 版本、部署方式、复现步骤、期望结果和实际结果。贴日志前请删掉 API Key、Cookie 和内网地址。
- 新功能：先开 issue 说明要解决的问题和使用场景，确认方向后再写代码，避免白费工夫。
- 安全问题不要公开提 issue，请按 [SECURITY.md](SECURITY.md) 私下报告。

## 本地开发

后端是 Rust（Axum、SQLx、PostgreSQL、Redis），控制台是 React + TypeScript + Vite，桌面客户端（`client/`）是 Tauri 2。

```bash
# 需要本地 PostgreSQL 和 Redis；Rust 进程不会自动读取 .env
export DATABASE_URL=postgres://openvair:openvair@localhost:5432/openvair
export REDIS_URL=redis://127.0.0.1/
export MASTER_KEY="$(openssl rand -hex 32)"
cargo run

npm --prefix web ci
npm --prefix web run dev
```

数据库迁移在 `migrations/`，服务启动时自动执行。设计文档在 [docs/design/](docs/design/)，部署文档在 [docs/](docs/)，宣传网站在 [site/](site/)。

## 提交代码

1. Fork 仓库，从 `main` 拉一个分支。
2. 保持一次 PR 只做一件事。数据库结构变化请新增迁移文件，不要修改已有迁移。
3. 提交前运行：

   ```bash
   cargo fmt --all
   cargo clippy --locked --all-targets -- -D warnings
   cargo clippy --locked --all-targets --features cloud -- -D warnings
   cargo test --locked
   npm --prefix web ci
   npm --prefix web run build
   git diff --check
   ```

   改动涉及数据库时，再运行 `scripts/` 下相关的 `test-*.sh`。改动桌面客户端时，按 [client/README.md](client/README.md) 在 `client/` 下验证。

4. 在 PR 描述里说明改了什么、为什么改、怎么验证的。

代码约定见 [AGENTS.md](AGENTS.md)。

## 许可

服务端和控制台按 [AGPL-3.0](LICENSE) 授权，桌面客户端（`client/`）按 [Apache-2.0](client/LICENSE) 授权。提交 PR 即表示你同意你的贡献按所在目录的许可证授权。
