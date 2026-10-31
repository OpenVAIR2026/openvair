[中文](self-hosting.md) · **English**

# Deployment and operations

Run commands in the deployment directory (`~/openvair` or the source repository). After editing `.env`, run `docker compose up -d` to apply it.

## Installation

Requires 64-bit Linux (x86_64 or arm64; Docker Desktop works for trials), Docker and Compose v2, and access to your upstream model APIs:

```bash
curl -fsSL https://raw.githubusercontent.com/OpenVAIR2026/openvair/main/install.sh | sh
```

The script installs to `~/openvair`, asks whether to allow LAN access (see [Choosing a deployment method](#choosing-a-deployment-method)), starts PostgreSQL 16, Redis 7 and OpenVAIR, then prints the URL and setup token. The database password and `MASTER_KEY` are generated automatically.

Build from source:

```bash
git clone https://github.com/OpenVAIR2026/openvair.git
cd openvair
OPENVAIR_IMAGE=openvair:local docker compose up -d --build
```

> [!WARNING]
> [Back up](#backup-and-restore) `MASTER_KEY`. Without it, channel keys can't be decrypted.

## Create the admin

Open the URL with `/setup` appended (e.g. `http://localhost:8080/setup`), enter the setup token, an email and a password (10–256 characters), and click **Create admin** (创建管理员). Or set `INITIAL_ADMIN_EMAIL` and `INITIAL_ADMIN_PASSWORD` in `.env` to create it on first start.

This is the instance's only account; everyone else uses API keys issued by the admin. Next, see [Quick start](../web/src/docs/content/en/quickstart.md).

## Choosing a deployment method

| Method | URL | Allow LAN access |
| --- | --- | --- |
| Local trial | `http://localhost:8080` | N |
| LAN | `http://<server-ip>:8080` | y |
| Public domain (recommended) | `https://your-domain` | N, plus a reverse proxy |

LAN: the script writes `OPENVAIR_BIND=0.0.0.0` and `COOKIE_SECURE=false`. HTTP only (see [limitations](#limitations-over-plain-http)); give the server a fixed IP on your router.

> [!WARNING]
> Don't choose LAN on a cloud server: `0.0.0.0:8080` exposes the console and keys to the internet in plain text, and ports published by Docker bypass host firewalls such as ufw.

Public domain:

1. Point a DNS A record (AAAA for IPv6) at the server's public IP.
2. Open TCP 80 (certificates and redirect) and 443.
3. Run the install script and answer N to LAN access.
4. [Install Caddy](https://caddyserver.com/docs/install) and add the config from the next section.
5. Open `https://your-domain/setup` to finish setup; teammates enter that domain in the client.

## Reverse proxy and HTTPS

OpenVAIR listens only on `127.0.0.1:8080` by default. The proxy must disable buffering and relax timeouts and the body size limit. Put the Caddy config (certificates are automatic) in `/etc/caddy/Caddyfile`, then `sudo systemctl reload caddy`:

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

Nginx: `proxy_buffering off`, `proxy_read_timeout 600s`, `client_max_body_size 51m`.

- **Client IP** (used for login rate limiting and audit): `X-Forwarded-For` is trusted only from `TRUSTED_PROXIES` (`X-Real-IP` is ignored); otherwise the TCP peer address is used. Default: `127.0.0.0/8,::1/128,10.0.0.0/8,172.16.0.0/12,192.168.0.0/16,fc00::/7`. Add a proxy on another public address, e.g. `TRUSTED_PROXIES=127.0.0.1/32,203.0.113.10/32`; don't trust arbitrary sources.
- **Security headers**: the server already sends HSTS, `X-Content-Type-Options: nosniff`, `Referrer-Policy` and a CSP (with `frame-ancestors`). Don't override them with looser values.

## Limitations over plain HTTP

Over `http://` on a non-local address, keys and conversations travel in plain text and can be sniffed on the same network. Also:

- **Login**: set `COOKIE_SECURE=false` (the script does this for LAN), or you'll be sent back to the login page right after logging in. Never set it on the public internet.
- **Copying keys**: the clipboard is unavailable; the full key appears at the top of the page. Click **Copy full API key** (复制完整 API Key).
- **Desktop client**: localhost, LAN, Tailscale (`100.64.0.0/10`) and domains like `.lan`, `.internal`, `.home.arpa` only show an unencrypted notice; other addresses require confirmation when adding the first key.

For internal HTTPS, use Caddy `tls internal` (teammates must trust its root certificate) or `tailscale serve`.

## Changing the IP address or domain

The gateway URL and setup instructions in the console default to the browser's address. If the admin usually opens the console via `localhost`, or the address is changing, enter the address teammates use under **System settings → Access URL** (系统管理 → 访问地址); the console, key pages and docs all switch to it.

| Situation | What to do |
| --- | --- |
| New IP, same domain | Point DNS at the new IP. Teammates change nothing. |
| New domain | Add DNS for it; in the Caddyfile use `old.example.com, new.example.com { ... }` and reload; remove the old domain once everyone has switched. Don't redirect: some agents and SDKs don't follow redirects, or turn POST into GET. |
| LAN IP changed | Teammates switch to the new IP. Fix the IP on your router. |
| HTTP to HTTPS | Set `COOKIE_SECURE` back to `true` (or delete it) and `OPENVAIR_BIND` back to `127.0.0.1`. HSTS makes browsers force HTTPS for a year, so the domain can't go back to HTTP. |
| Proxy moved to another machine | If it's outside localhost and private ranges, add it to `TRUSTED_PROXIES`. |

After the change:

1. Update **System settings → Access URL** to the new address.
2. Log in to the console again (login state is per domain).
3. The desktop client shows the new address under **Settings** (设置) if the old one is still reachable during the transition, or click **Change server** (更换服务器) to enter it. The client verifies the new address is the same server using its local keys, then keeps all keys and moves connected agents over.
4. Update the Base URL in manually configured agents and scripts.

## Pulling images from servers in mainland China

`ghcr.io` may fail from mainland China (Docker Hub mirrors only help with PostgreSQL and Redis). Options:

- [Configure a proxy](https://docs.docker.com/engine/daemon/proxy/) for the Docker daemon, then rerun the install script.
- On a machine that can reach it, `docker pull`, then `docker save ghcr.io/openvair2026/openvair:latest -o openvair.tar`; on the server, `docker load -i openvair.tar`, then `docker compose up -d`.
- Build from source (npm and crates.io dependencies must be downloadable).

## Upgrading

**[Back up](#backup-and-restore) first** and read the release notes, then rerun the install script (it keeps `.env` and updates the compose file and image):

```bash
curl -fsSL https://raw.githubusercontent.com/OpenVAIR2026/openvair/main/install.sh | sh
```

Manually: `docker compose pull && docker compose up -d`; from source: `git pull`, then rebuild. Migrations run on startup and can span versions, but can't be rolled back.

## Backup and restore

- **PostgreSQL**: all business data.
- **`MASTER_KEY`**: store it separately; without it, channel keys must be re-entered one by one. It lives in the volume `secrets`; view it with `docker compose exec openvair cat /run/openvair/master_key`.
- Redis holds only temporary data; no backup needed.

```bash
docker compose exec -T postgres pg_dump -U openvair -d openvair --format=custom > openvair-$(date +%F).dump
```

To restore to a new deployment, put the original `MASTER_KEY=...` in `.env` **before the first start** (a deployment that has already started keeps the value in its volume), then:

```bash
docker compose up -d postgres
docker compose exec -T postgres pg_restore -U openvair -d openvair --clean --if-exists --no-owner < openvair-2026-01-01.dump
docker compose up -d
```

## Monitoring and the admin password

Health check: `GET /healthz`; logs: `docker compose logs -f openvair`; channel status: [Channel monitoring](../web/src/docs/content/en/monitoring.md) (no alert emails without an email service). To reset the admin password (10–256 characters; all sessions are signed out):

```bash
docker compose exec openvair openvair admin reset-password admin@example.com
```

## Data growth

Request logs expire automatically; usage records (`usage_events`) are never cleaned up. Delete old ones by hand if needed, e.g. keep 180 days (earlier stats are lost):

```bash
docker compose exec -T postgres psql -U openvair -d openvair \
  -c "DELETE FROM usage_events WHERE created_at < now() - interval '180 days'"
```

## Limitations

- **Single instance only**: concurrency control lives in process memory. Don't scale to multiple replicas.
- **Channels can reach internal networks by default**: if untrusted people can access the console, set `ALLOW_PRIVATE_UPSTREAMS=false`. See [Channel address restrictions](../web/src/docs/content/en/privacy.md#channel-address-restrictions).

## Common issues

| Symptom | Fix |
| --- | --- |
| Can't find the setup token | Printed only while no admin exists, regenerated on every start. Run `docker compose restart openvair`, then the command below. |
| Other LAN machines can't connect | `docker compose ps` should show `0.0.0.0:8080->8080/tcp`; check the firewall and router guest isolation. |
| Sent back to login right after logging in | See [Limitations over plain HTTP](#limitations-over-plain-http). |
| Streaming cuts off or arrives all at once | See [Reverse proxy and HTTPS](#reverse-proxy-and-https). |

```bash
docker compose logs --no-log-prefix openvair | grep -A2 "setup token" | tail -1
```

**Uninstalling**: `docker compose down` removes only containers; `docker compose down -v` deletes the database and `MASTER_KEY` permanently, so [back up](#backup-and-restore) first.
