# 安全策略

## 报告漏洞

请不要在公开 issue、讨论区或 PR 中披露安全问题。

请通过 GitHub 的 [私下报告漏洞](https://github.com/OpenVAIR2026/openvair/security/advisories/new) 提交，或发邮件到 [w821881341@gmail.com](mailto:w821881341@gmail.com)。报告内容包括：

- 受影响的版本和部署方式；
- 问题描述和影响范围；
- 复现步骤或概念验证；
- 你建议的修复方式（如有）。

我们会在 3 个工作日内确认收到，确认问题后尽快发布修复版本和安全公告，并在公告中致谢（如果你愿意）。

## 支持的版本

只有最新的发布版本会收到安全修复。请及时升级。

## 部署建议

- 通过 HTTPS 暴露 OpenVAIR，并设置 `COOKIE_SECURE=true`。
- 使用随机生成的 `MASTER_KEY` 和数据库密码，并把 `MASTER_KEY` 和数据库备份分开保存。
- 不要把 PostgreSQL 和 Redis 暴露到公网。
- 不信任的人能访问控制台时，设置 `ALLOW_PRIVATE_UPSTREAMS=false`，禁止渠道指向内网地址。
