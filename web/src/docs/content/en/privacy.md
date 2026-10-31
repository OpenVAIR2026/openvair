All data stays in PostgreSQL and Redis on your own server. OpenVAIR only forwards requests to your configured upstream channels and sends codes and alerts through the sender mailbox under **Email settings** (邮箱设置); nothing goes anywhere else.

## What is stored

### Request data

| Data | Stored? | Purpose |
| --- | --- | --- |
| Request and response bodies (conversation content) | ❌ **Not stored** | — |
| Model name, protocol, HTTP status, channel | ✅ | Usage analytics, troubleshooting |
| Duration, first-token time, timing breakdown, channel attempts | ✅ | Performance stats, request details |
| Token usage and cost | ✅ | Quotas, cost tracking |
| Input character count (no content) | ✅ | Metering check |
| Monitoring and check results | ✅ | Channel monitoring |

Usage records aren't cleaned up automatically; delete them by hand if needed. See [Self-hosting and operations](https://github.com/OpenVAIR2026/openvair/blob/main/docs/self-hosting.en.md#data-growth).

### Account and credentials

| Data | Stored? | Purpose |
| --- | --- | --- |
| Admin login email and account name | ✅ | Login, notifications |
| Login password | ✅ Hash only | Login |
| User-Agent and last activity of signed-in devices | ✅ | **Signed-in devices** (登录设备) list |
| Activity log | ✅ Latest 200 entries viewable | Account security |
| Upstream provider keys | ✅ Encrypted | Forwarding requests |
| API keys | ✅ Hash + encrypted copy | Authentication, copying again |

## How keys are protected

- **Upstream keys**: AES-256-GCM encrypted with the server's `MASTER_KEY`, decrypted only when calling the upstream; **never** sent to any client (console, desktop client, your tools).
- **API keys**: authentication compares hashes; the encrypted copy is decrypted only when the admin clicks **Copy** (复制), and each copy is logged in the activity log; **not** forwarded upstream.
- Full keys never appear in logs or error messages.

> [!IMPORTANT]
> Store `MASTER_KEY` separately from database backups. Without it, no upstream key in a backup can be decrypted.

## Security recommendations

- **One key per person and device**, with [quotas](/docs/limits).
- **Don't hard-code keys or commit them to Git.**
- **Revoke a key immediately if it may have leaked**; disabling doesn't invalidate the old key. See [Disable, reset and revoke](/docs/keys#disable-reset-and-revoke).
- **Review signed-in devices and the [activity log](/docs/account#activity-log) regularly.**
- **Use HTTPS for the console.** Login cookies are HTTPS-only by default (`COOKIE_SECURE=true`); don't turn this off on a public network.
- **If untrusted people can reach the console**, set `ALLOW_PRIVATE_UPSTREAMS=false` on the server so channels can't point to `localhost`, private IPs or domains resolving to private addresses (allowed by default).
