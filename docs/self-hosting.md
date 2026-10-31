**中文** · [English](self-hosting.en.md)

# 部署与运维

命令在部署目录（`~/openvair` 或源码仓库）执行；改 `.env` 后执行 `docker compose up -d` 生效。

## 安装

需要 64 位 Linux（x86_64 或 arm64，Docker Desktop 可试用）、Docker 和 Compose v2，并能访问上游模型 API：

```bash
curl -fsSL https://raw.githubusercontent.com/OpenVAIR2026/openvair/main/install.sh | sh
```

脚本安装到 `~/openvair`，询问是否允许局域网访问（见[选择部署方式](#选择部署方式)），启动 PostgreSQL 16、Redis 7 和 OpenVAIR 后打印访问地址和初始化令牌；数据库密码和 `MASTER_KEY` 自动生成。

从源码构建：

```bash
git clone https://github.com/OpenVAIR2026/openvair.git
cd openvair
OPENVAIR_IMAGE=openvair:local docker compose up -d --build
```

> [!WARNING]
> 请[备份](#备份和恢复) `MASTER_KEY`，丢失后渠道 Key 无法解密。

## 创建管理员

打开「访问地址/setup」（如 `http://localhost:8080/setup`），填入初始化令牌、邮箱和密码（10–256 个字符），点「创建管理员」。或在 `.env` 设置 `INITIAL_ADMIN_EMAIL`、`INITIAL_ADMIN_PASSWORD`，首次启动时创建。

这是实例唯一的帐号，其他人用管理员发的 API Key。下一步见[快速开始](../web/src/docs/content/quickstart.md)。

## 选择部署方式

| 方式 | 地址 | 允许局域网访问 |
| --- | --- | --- |
| 本机试用 | `http://localhost:8080` | N |
| 局域网 | `http://服务器IP:8080` | y |
| 公网域名（推荐） | `https://你的域名` | N，加反向代理 |

局域网：脚本写入 `OPENVAIR_BIND=0.0.0.0`、`COOKIE_SECURE=false`；只能用 HTTP（见[限制](#用-http-访问时的限制)），建议在路由器上固定 IP。

> [!WARNING]
> 云服务器不要选局域网：`0.0.0.0:8080` 会向公网明文暴露控制台和 Key，且 Docker 发布的端口绕过 ufw 等主机防火墙。

公网域名：

1. DNS A 记录（IPv6 用 AAAA）指向服务器公网 IP。
2. 放行 TCP 80（证书和跳转）和 443。
3. 运行安装脚本，局域网回答 N。
4. [安装 Caddy](https://caddyserver.com/docs/install)，写入下一节配置。
5. 打开 `https://你的域名/setup` 初始化，同事在客户端填该域名。

## 反向代理和 HTTPS

OpenVAIR 默认只监听 `127.0.0.1:8080`。代理需关闭缓冲，放宽超时和请求体上限。Caddy 配置（自动申请证书）写入 `/etc/caddy/Caddyfile` 后 `sudo systemctl reload caddy`：

```caddy
openvair.example.com {
    request_body {
        max_size 51MiB
    }
    reverse_proxy 127.0.0.1:8080 {
        flush_interval -1
        transport http {
            read_timeout 10m
            write_timeout 10m
        }
    }
}
```

Nginx：`proxy_buffering off`、`proxy_read_timeout 600s`、`client_max_body_size 51m`。

- **客户端 IP**（用于登录限流和审计）：只采信 `TRUSTED_PROXIES` 发来的 `X-Forwarded-For`（忽略 `X-Real-IP`），否则用 TCP 对端地址。默认 `127.0.0.0/8,::1/128,10.0.0.0/8,172.16.0.0/12,192.168.0.0/16,fc00::/7`；代理在其他公网地址时加入，如 `TRUSTED_PROXIES=127.0.0.1/32,203.0.113.10/32`，不要信任任意来源。
- **安全响应头**：服务端已发 HSTS、`X-Content-Type-Options: nosniff`、`Referrer-Policy`、CSP（含 `frame-ancestors`），代理勿用更宽松的值覆盖。

## 用 HTTP 访问时的限制

用 `http://` 访问非本机地址时，Key 和对话明文传输，同网络可抓包。此外：

- **登录**：须设 `COOKIE_SECURE=false`（选局域网时脚本自动设置），否则登录后立即退回登录页。公网不要这样设置。
- **复制 Key**：剪贴板不可用，完整 Key 显示在页面上方，点「复制完整 API Key」。
- **桌面客户端**：本机、局域网、Tailscale（`100.64.0.0/10`）和 `.lan`、`.internal`、`.home.arpa` 等仅提示未加密，其他地址添加首把 Key 时需确认。

内网 HTTPS 可用 Caddy `tls internal`（同事需信任其根证书）或 `tailscale serve`。

## 更换 IP 地址或域名

控制台的网关地址和接入说明默认取自浏览器地址。管理员常用 `localhost` 打开控制台或要换地址时，在「系统管理 → 访问地址」填写同事使用的地址，控制台、Key 页面和文档都会改用它。

| 情况 | 操作 |
| --- | --- |
| 只换 IP，域名不变 | DNS 改到新 IP，同事无需改动。 |
| 换域名 | 添加新域名 DNS；Caddyfile 写成 `old.example.com, new.example.com { ... }` 并重新加载；所有人切换后再删旧域名。不要重定向：部分 Agent 和 SDK 不跟随重定向，或把 POST 改成 GET。 |
| 局域网 IP 变了 | 同事改用新 IP。建议在路由器上固定 IP。 |
| HTTP 改 HTTPS | `COOKIE_SECURE` 改回 `true`（或删除），`OPENVAIR_BIND` 改回 `127.0.0.1`。HSTS 使浏览器一年内强制 HTTPS，域名不能再退回 HTTP。 |
| 代理换到其他机器 | 不在本机和私有网段时，加入 `TRUSTED_PROXIES`。 |

换地址后：

1. 「系统管理 → 访问地址」改为新地址。
2. 控制台需重新登录（登录状态按域名保存）。
3. 桌面客户端会在「设置」提示新地址（旧地址过渡期须可访问），也可点「更换服务器」填写。客户端用本机 Key 验证新地址是同一台服务器后，保留全部 Key 并把已接入的 Agent 改到新地址。
4. 手动配置的 Agent 和脚本修改 Base URL。

## 国内服务器拉取镜像

`ghcr.io` 在国内可能拉取失败（Docker Hub 加速只对 PostgreSQL、Redis 有效）。可以：

- 为 Docker 守护进程[配置代理](https://docs.docker.com/engine/daemon/proxy/)后重新运行安装脚本。
- 在能访问的机器上 `docker pull` 后 `docker save ghcr.io/openvair2026/openvair:latest -o openvair.tar`，到服务器 `docker load -i openvair.tar` 再 `docker compose up -d`。
- 从源码构建（需能下载 npm 和 crates.io 依赖）。

## 升级

**先[备份](#备份和恢复)**并阅读发布说明，再运行安装脚本（保留 `.env`，更新 compose 文件和镜像）：

```bash
curl -fsSL https://raw.githubusercontent.com/OpenVAIR2026/openvair/main/install.sh | sh
```

手动：`docker compose pull && docker compose up -d`；源码：`git pull` 后重新构建。迁移启动时自动执行，可跨版本，不能回退。

## 备份和恢复

- **PostgreSQL**：全部业务数据。
- **`MASTER_KEY`**：单独保存，丢失后渠道 Key 只能逐个重填。位于卷 `secrets`，查看：`docker compose exec openvair cat /run/openvair/master_key`。
- Redis 只有临时数据，无需备份。

```bash
docker compose exec -T postgres pg_dump -U openvair -d openvair --format=custom > openvair-$(date +%F).dump
```

恢复到新部署：**首次启动前**在 `.env` 写入原 `MASTER_KEY=...`（已启动过的部署沿用卷中的值），然后：

```bash
docker compose up -d postgres
docker compose exec -T postgres pg_restore -U openvair -d openvair --clean --if-exists --no-owner < openvair-2026-01-01.dump
docker compose up -d
```

## 监控和管理员密码

健康检查 `GET /healthz`，日志 `docker compose logs -f openvair`，渠道状态见[渠道监控](../web/src/docs/content/monitoring.md)（未配置邮件服务时不发告警）。重置管理员密码（10–256 个字符，所有会话随即退出）：

```bash
docker compose exec openvair openvair admin reset-password admin@example.com
```

## 数据增长

请求日志到期自动清理，用量记录（`usage_events`）不清理。可手动删除，如保留 180 天（更早的统计随之消失）：

```bash
docker compose exec -T postgres psql -U openvair -d openvair \
  -c "DELETE FROM usage_events WHERE created_at < now() - interval '180 days'"
```

## 限制

- **只能单实例**：并发控制在进程内存中，不要扩到多个副本。
- **渠道默认可访问内网**：不信任的人能访问控制台时设 `ALLOW_PRIVATE_UPSTREAMS=false`，见[渠道地址限制](../web/src/docs/content/privacy.md#渠道地址限制)。

## 常见问题

| 现象 | 处理 |
| --- | --- |
| 找不到初始化令牌 | 仅在没有管理员时打印，每次启动重新生成。`docker compose restart openvair` 后执行下方命令。 |
| 局域网其他电脑打不开 | `docker compose ps` 应显示 `0.0.0.0:8080->8080/tcp`；检查防火墙和路由器访客隔离。 |
| 登录后马上回到登录页 | 见[用 HTTP 访问时的限制](#用-http-访问时的限制)。 |
| 流式回复中断或最后一次性出现 | 见[反向代理和 HTTPS](#反向代理和-https)。 |

```bash
docker compose logs --no-log-prefix openvair | grep -A2 "setup token" | tail -1
```

**卸载**：`docker compose down` 只删容器；`docker compose down -v` 会永久删除数据库和 `MASTER_KEY`，先[备份](#备份和恢复)。
