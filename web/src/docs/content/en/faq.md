## About the product

### Does OpenVAIR provide model quota?

No. Quota and cost come from your channels; OpenVAIR only forwards, routes, limits and tracks usage.

### How is OpenVAIR different from CC Switch?

CC Switch switches tool configs on one computer; OpenVAIR is a server gateway where every device shares one address, with key issuing, quotas and channel monitoring. They work together; see [CC Switch](/docs/other-clients#cc-switch).

### Which models are supported?

Chat models served over the OpenAI-compatible, Anthropic Messages or Gemini protocol, including services on your machine or internal network. Embeddings, image, audio and similar aren't supported; see [Supported endpoints](/docs/api#supported-endpoints).

### Can I send OpenAI-format requests to Claude?

No. OpenVAIR doesn't convert protocols; see [Protocols must match](/docs/concepts#protocols-must-match). Use a channel that serves Claude over the OpenAI-compatible protocol (many relays do).

## Using OpenVAIR

### Do clients need a restart after I change channel or key settings?

No. Changes take effect on the server immediately.

### Can I give team members console accounts?

No. Give each member a key and note who it's for in **Holder** (持有人); members check quota on [View my key](/docs/my-key) without signing in.

### Can I see what members send with their keys?

No. OpenVAIR stores only metadata, not conversation content; see [Data and privacy](/docs/privacy).

### What if I forget the admin password?

See [Forgot password](/docs/account#forgot-password).

## Cost

### Are cost limits accurate?

Costs are estimates and quota is checked before a request starts, so usage can slightly exceed a limit; see [How cost is calculated](/docs/limits#how-cost-is-calculated).

### Do monitoring and connection tests cost money?

| Action | Cost |
| --- | --- |
| **Test connection** (测试连接) when adding a channel, availability checks | 1 output token each, tiny |
| Service checkups | Small; estimate shown in settings |
| Long-context tests | Higher; estimate shown first |
| Passive checks, **Test connection** in a key's setup guide | Free |

## Security

### Is it safe to put provider keys in OpenVAIR?

Provider keys are encrypted on your server and never sent to clients; devices only hold API keys you can revoke individually. See [Data and privacy](/docs/privacy).
