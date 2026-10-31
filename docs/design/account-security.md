# 帐号安全

OpenVAIR 是单租户的：一个实例只有一个管理员帐号，在首次启动时通过 `/setup` 或 `INITIAL_ADMIN_EMAIL`、`INITIAL_ADMIN_PASSWORD` 创建，没有注册入口。本文描述这个帐号的登录和安全设置。

## 设置页

帐号设置分为几组，每一项原地修改、单独保存：

- **帐号信息**：帐号名称、登录邮箱、登录密码、登录设备。
- **连接信息**：API Key 前缀。
- **告警邮件**：选择哪些监控告警发送到登录邮箱。

## 用户与帐号

管理员用户只拥有一个帐号，系统不提供成员邀请、角色或切换帐号。登录后通过会话找到用户及其帐号，所有帐号级资源仍按 `account_id` 隔离。管理员同时是超级管理员（`require_super_admin`），可以使用「系统管理」中的路由设置、模型管理、发信邮箱等功能。

迁移 `0039_single_user_accounts.sql` 为 `memberships` 增加“一个用户一个帐号”“一个帐号一个用户”的唯一约束，`role` 列固定为 `owner`，并删除不再使用的 `invitations` 表。

## 修改邮箱

`POST /api/me/email-verification` 需要新邮箱和当前密码，发送 6 位验证码；`POST /api/me/email-verification/confirm` 提交验证码完成修改。需要先在「系统管理」中配置发信邮箱（SMTP）。

- 验证码以 Argon2 哈希保存，10 分钟有效，最多尝试 5 次，并绑定发起请求的会话和目标邮箱；改动目标邮箱后需重新获取。
- 当前密码校验与修改密码共用“每用户 10 分钟 10 次”的限制；发送另有 60 秒冷却、每小时 10 次、同一收件地址每小时 5 次的限制。
- 修改成功后退出该用户其他设备上的登录，并向原邮箱发送通知（发送失败时不影响修改）。

## 修改密码

`POST /api/me/password`（`current_password`、`new_password`）：

- 新密码 10–256 字节，且不能与当前密码相同；当前密码校验与修改邮箱共用尝试次数限制。
- 只替换已验证的密码哈希，并发修改返回 409。
- 成功后退出其他设备上的登录，作废未完成的邮箱修改，记录审计 `user.password_updated`，并向登录邮箱发送通知。

## 找回密码

已配置发信邮箱时，可以在登录页通过邮件验证码重置密码：`POST /api/auth/password-reset`（`email`）发送重置验证码；`POST /api/auth/password-reset/confirm`（`request_id`、`email`、`code`、`new_password`）完成重置。

- 无论邮箱是否存在，请求都返回相同结构的 `request_id`；验证码哈希在查询用户之前生成，邮件在后台发送，响应时间不暴露邮箱是否存在。
- 限制：同一客户端地址每小时 20 次，同一邮箱 60 秒冷却、每小时 5 次；每个验证码 10 分钟有效、最多尝试 5 次。
- 重置成功后退出该用户所有设备上的登录，作废未完成的邮箱修改和登录失败计数，记录审计 `user.password_reset` 并发送通知。

没有配置发信邮箱时，在服务器上用命令行重置，见 [部署与运维](../self-hosting.md#重置管理员密码)：

```bash
docker compose exec openvair openvair admin reset-password admin@example.com
```

## 登录保护

- 登录失败按邮箱（15 分钟 10 次）和客户端地址（15 分钟 50 次）计数，超出后返回 429“登录失败次数过多，请 15 分钟后再试”；成功登录清除该邮箱的计数。
- 客户端地址只在请求来自 `TRUSTED_PROXIES` 中的反向代理时才取自 `X-Forwarded-For`（最右侧不属于可信代理的地址），否则使用 TCP 对端地址，见 [部署与运维](../self-hosting.md)。
- 不存在的邮箱同样计数，并使用固定哈希完成一次密码校验，失败响应与密码错误一致；控制台统一显示“邮箱或密码不正确”。
- 密码校验在阻塞线程池执行。Redis 不可用时登录限制放行并记录日志，避免管理员无法进入控制台。

## 登录设备

- 会话记录浏览器标识（最多 256 字符）和最近活动时间；最近活动在 `GET /api/me` 时更新，至多每 5 分钟一次。
- `GET /api/me/sessions` 列出当前用户未过期的会话，不返回令牌或哈希；`DELETE /api/me/sessions/{id}` 退出其他设备（当前设备请使用“退出登录”）；`POST /api/me/sessions/revoke-others` 退出所有其他设备。均记录审计。

## 迁移

`0038_account_security.sql` 新增 `password_resets`，并为 `sessions` 增加可空的 `user_agent`、`last_seen_at`。

## 验证

```bash
bash scripts/test-account-security.sh   # 临时 PostgreSQL、Redis 与模拟 SMTP
npm --prefix web run test:settings
```
