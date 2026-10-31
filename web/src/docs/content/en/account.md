The console has one account: the admin created on the `/setup` page at first install. Everyone else uses an [API Key](/docs/keys).

## Account info

In **Account settings** (帐号设置) you can change:

- **Account name** (帐号名称): shown in the console.
- **Sign-in email** (登录邮箱): requires your current password and a 6-digit code sent to the new address (valid 10 minutes).; needs a sender email under **System → Email settings** (系统管理 → 邮箱设置).
- **Password** (登录密码): 10–256 characters, different from the current one.

After changing email or password, other devices are signed out; the current one stays signed in. 10 failed sign-ins for the same email within 15 minutes temporarily block sign-in; retry after 15 minutes.

### Forgot password

With a sender email configured, click **Forgot password?** (忘记密码？) on the sign-in page and reset with an email code. Otherwise, run this on the server and enter a new password (10–256 characters) when prompted:

```bash
openvair admin reset-password admin@example.com
# Docker Compose deployment:
docker compose exec openvair openvair admin reset-password admin@example.com
```

All devices are signed out after the reset.

## Signed-in devices

**Signed-in devices** (登录设备) lists signed-in browsers and their last activity; use **Sign out** (退出) for one device or **Sign out other devices** (退出其他设备).

## API Key prefix

Default `openvair`. 1–32 letters, digits, hyphens or underscores, starting with a letter or digit. Only affects keys created afterward.

## Alert emails

[Channel monitoring](/docs/monitoring) emails your sign-in address when it finds a problem; needs a sender email. The three types toggle separately; by default only channel availability is on.

| Alert | When it's sent |
| --- | --- |
| **Service checkup** (服务体检) | A service checkup confirms a problem |
| **Metering anomaly** (计量异常) | The metering check confirms inflated usage or a hidden system prompt (one failed exact comparison, or two consecutive failed estimates). If no service checkup can confirm it, sent as soon as real traffic reveals it |
| **Channel availability** (渠道可用性) | A channel is unavailable for over 30 minutes, or the upstream rejects its key (401–403). Another email on recovery. At most one per channel per day; brief blips don't trigger it |

Service checkup and metering alerts are sent once when a problem appears and re-evaluated after it clears; metering anomalies share one record between checkups and real traffic, so one problem sends one email.

## System

| Page | Purpose |
| --- | --- |
| **Routing settings** (路由设置) | Routing parameters for the whole instance |
| **Model management** (模型管理) | Maintain the model catalog |
| **Email settings** (邮箱设置) | Sender email for codes and alerts |
| **System status** (系统状态) | Service health |

## Activity log

Records the last 200 actions: creating, editing and deleting channels; creating, copying, editing and revoking API Keys; and monitoring setting changes.

![The Activity log page](/docs-img/account-audit.webp)
