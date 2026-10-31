# OpenVAIR Agent Guide

## 项目概览

- Rust 后端位于 `src/`，使用 Axum、SQLx、PostgreSQL 和 Redis。
- 前端控制台位于 `web/`，使用 React、TypeScript、Vite 和 TanStack Query。
- 桌面客户端位于 `client/`，使用 Tauri 2，见 `client/README.md`。
- 数据库迁移位于 `migrations/`，应用启动时通过 `sqlx::migrate!` 执行。
- 默认构建是单租户的 OpenVAIR。`cloud` feature 打开托管服务才有的功能（开放注册、公开验真、跨帐号共享 Key）；版本差异只通过 `edition::CLOUD` 判断，用普通 `if` 而不是 `#[cfg]`，保证两种构建编译同一份代码。

## 修改和验证

- 修改前先查看 `git status`，不要覆盖已有未提交改动。
- Rust 修改后运行：

  ```bash
  cargo fmt --all
  cargo clippy --locked --all-targets -- -D warnings
  cargo clippy --locked --all-targets --features cloud -- -D warnings
  cargo test --locked
  ```

- 依赖数据库的测试用 `scripts/test-*.sh` 运行，脚本会临时启动隔离的 PostgreSQL 和 Redis，需要本机装有 PostgreSQL 工具和 `redis-server`。
- 前端修改后运行：

  ```bash
  npm --prefix web ci
  npm --prefix web run build
  ```

- 提交前运行 `git diff --check`，确认没有调试代码、真实密钥或临时测试数据。
- 不要在测试中写入真实上游 API Key；使用临时帐号、临时渠道和模拟凭据，并在测试结束后清理。

## 代码约定

- API 权限沿用现有 `current_user` 和 `require_super_admin` 机制。OpenVAIR 只有初始化时创建的一个管理员和一个帐号，不提供成员邀请和角色。
- 所有帐号级资源查询都必须限制 `account_id`；托管版是多租户的，单租户构建也不能省略这个条件。
- API Key 撤销使用 `revoked_at`，撤销记录不应出现在普通 Key 列表中，也不能重新启用。
- 渠道和 API Key 的启停使用 `enabled`；网关路由和鉴权查询必须尊重该字段。
- 新增数据库字段时创建新的迁移文件，不修改已经发布的迁移。迁移只向前执行。
- 密钥只能加密保存或保存哈希，日志、审计元数据和错误信息不得输出完整密钥。

## 提交

- 提交信息使用简洁的动词开头，例如 `feat:`、`fix:`、`docs:`。
- 不要提交 `.env`、数据库密码、`MASTER_KEY` 或任何真实上游凭据。
