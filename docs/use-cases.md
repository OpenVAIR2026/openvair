一个 OpenVAIR 实例服务一个人或一个团队，只有一个管理员帐号。成员不注册，按 **Key** 分配额度；上游费用由供应商直接与你结算。

| 你是… | 痛点 | 做法 |
| --- | --- | --- |
| [小团队](#小团队) | 怕超支、怕 Key 外泄 | 每人一把 Key，单独限额 |
| [科研课题组](#科研课题组) | 经费有限、学生流动 | 按人或课题发 Key，毕业即撤销 |
| [个人开发者](#个人开发者) | 工具多，渠道一挂就停工 | 一个地址，自动切换渠道 |
| [应用和后端开发者](#应用和后端开发者) | 依赖单一供应商 | 故障切换、调用明细 |
| [拿到 Key 的成员](#拿到-key-的成员) | 不会用、不知余额 | 无需帐号，随时查额度 |

## 小团队

团队用 Claude Code、Codex 等 Agent，你管预算，不想外发供应商 Key。

1. [部署 OpenVAIR](/deploy)，接入公司渠道。
2. 为每位成员[创建 Key](/docs/keys)，「持有人」填姓名，设好[限额](/docs/limits)。
3. 把地址和 Key 发给对方，对方在 [OpenVAIR Desktop](/download) 粘贴 Key、点「接入」。

- 用超只限制他自己的 Key；每日上限次日恢复，总费用上限需手动调整。
- 离职或外泄时[停用](/docs/keys#停用重置和撤销)（可恢复）或撤销（永久）。
- 使用分析按 Key 筛选，成员在 [Key 查询页](/docs/my-key)自查。
- 渠道持续不可用时发[告警邮件](/docs/account#告警邮件)。

## 科研课题组

导师管经费，可能有内网模型或自建 GPU，学生每年进出。

1. 在实验室服务器部署，仅内网使用可[用局域网地址访问](/deploy#选择部署方式)。
2. 把官方 API、中转站和内网服务[添加为渠道](/docs/channels)，内网为主、外部为备。
3. 按人或课题[创建 Key](/docs/keys)，设好费用上限和可用模型。
4. 学生用 [OpenVAIR Desktop](/download) 接入，或把 SDK 的 Base URL 换成 OpenVAIR（见[其他客户端与 SDK](/docs/other-clients)）。

- 按 Key 查看每人、每个课题的费用；毕业撤销即可，上游 Key 不在学生手里。
- 内网忙或宕机时自动改走外部渠道。
- 不存正文，见[数据与隐私](/docs/privacy)。

## 个人开发者

多台设备用 Claude Code、Codex、Gemini CLI，有多个模型来源。

1. 在常开的机器上[部署 OpenVAIR](/deploy)。
2. 把所有来源[添加为渠道](/docs/channels)，主力渠道优先级数字设小。
3. 每台设备[创建一把 Key](/docs/keys)，如「MacBook」。
4. 用 [OpenVAIR Desktop](/docs/desktop) 一键配置，或按[接入工具](/docs/claude-code)手动配置。

- 换供应商只改渠道；限流或故障时[自动切换](/docs/routing)。
- 上游 Key 不落地，设备丢了撤销其 Key 即可。

## 应用和后端开发者

产品、内部工具或 CI 调用模型，需要容错并按应用统计。

1. 为每个应用或环境（生产、测试、CI）创建 Key，按需限制模型、频率和并发。
2. 把 SDK 的 Base URL 和 Key 换成 OpenVAIR 的，见[其他客户端与 SDK](/docs/other-clients) 和 [API 参考](/docs/api)。
3. 为同一模型配置多个同协议渠道并[设置优先级](/docs/routing)。

- 继续用官方 SDK，换供应商不改代码。
- 限流、5xx、连接失败和[流式首字超时](/docs/routing#流式首字超时)自动切换，客户端看不到失败的尝试。
- 调用明细含耗时、首字时间、Token 和各渠道尝试结果。

## 拿到 Key 的成员

1. 安装 [OpenVAIR Desktop](/download)，填入服务器地址和 Key 一键配置；或按[接入工具](/docs/claude-code)手动填写。
2. 在 [Key 查询页](/docs/my-key)查看可用模型、剩余额度和最近用量。

无需注册。报错信息末尾附有 Key 查询页地址，常见报错见[问题排查](/docs/troubleshooting)。
