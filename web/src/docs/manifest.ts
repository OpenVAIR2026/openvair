// The order here is the reading order: the sidebar, previous/next links and search all follow it.
// These pages are for people using a deployment; deploying and choosing OpenVAIR live in docs/ and site/.
// site/ renders the same pages, so links here should stay within /docs or point at absolute URLs.
const sources = {
  zh: import.meta.glob<string>('./content/*.md', { eager: true, query: '?raw', import: 'default' }),
  en: import.meta.glob<string>('./content/en/*.md', { eager: true, query: '?raw', import: 'default' }),
}

/** Docs are written in Chinese first; every page has an English twin under content/en/ with the same slug. */
export type DocLang = keyof typeof sources

export interface DocPage { slug: string; title: string; description: string; group: string; groupId: string; body: string }

type Text = [title: string, description: string]

const outline: { id: string; group: Record<DocLang, string>; pages: [slug: string, zh: Text, en: Text][] }[] = [
  { id: 'start', group: { zh: '开始', en: 'Get started' }, pages: [
    ['', ['欢迎使用 OpenVAIR', '这台 OpenVAIR 能帮你做什么，以及从哪里开始。'], ['Welcome to OpenVAIR', 'What this OpenVAIR can do for you, and where to start.']],
    ['quickstart', ['快速开始', '接入第一个渠道、发出第一把 Key，在 Claude Code 里发出第一次请求。'], ['Quick start', 'Add your first channel, issue your first key and send your first request from Claude Code.']],
    ['concepts', ['核心概念', '管理员、渠道、API Key、模型名和路由分别是什么。'], ['Core concepts', 'What the admin, channels, API keys, model names and routing are.']],
  ] },
  { id: 'tools', group: { zh: '接入工具', en: 'Connect tools' }, pages: [
    ['desktop', ['OpenVAIR Desktop', '填入服务器地址和 Key，一键写好本机 Agent 配置。'], ['OpenVAIR Desktop', 'Enter the server address and a key to configure local agents in one click.']],
    ['claude-code', ['Claude Code', '让 Claude Code 通过 OpenVAIR 调用模型。'], ['Claude Code', 'Point Claude Code at OpenVAIR.']],
    ['codex', ['Codex', '在 Codex 中添加 OpenVAIR 作为模型供应商。'], ['Codex', 'Add OpenVAIR as a model provider in Codex.']],
    ['gemini-cli', ['Gemini CLI', '让 Gemini CLI 通过 OpenVAIR 调用模型。'], ['Gemini CLI', 'Point Gemini CLI at OpenVAIR.']],
    ['other-clients', ['其他客户端与 SDK', 'CC Switch、聊天客户端，以及 OpenAI、Anthropic SDK。'], ['Other clients and SDKs', 'CC Switch, chat apps, and the OpenAI and Anthropic SDKs.']],
    ['api', ['API 参考', '网关支持的接口、鉴权方式和请求示例。'], ['API reference', 'Supported endpoints, authentication and request examples.']],
  ] },
  { id: 'channels', group: { zh: '渠道', en: 'Channels' }, pages: [
    ['channels', ['添加和管理渠道', '接入官方 API、中转站或内网服务。'], ['Add and manage channels', 'Connect official APIs, relay services or internal model servers.']],
    ['models', ['模型名与映射', '决定客户端能用哪些模型名，以及它们对应上游的哪个模型。'], ['Model names and mapping', 'Which model names clients can use, and which upstream model each one maps to.']],
    ['routing', ['备用渠道与故障切换', '一个渠道出问题时，请求如何自动改走另一个。'], ['Fallback channels and failover', 'How requests move to another channel when one has problems.']],
  ] },
  { id: 'keys', group: { zh: 'API Key', en: 'API keys' }, pages: [
    ['keys', ['创建和管理 API Key', '为设备、同事或应用分发独立的 Key。'], ['Create and manage API keys', 'Issue separate keys for devices, colleagues and apps.']],
    ['limits', ['限额与费用', '限制每把 Key 的模型、频率、Token 和费用。'], ['Limits and cost', 'Limit the models, rate, tokens and spend of each key.']],
    ['my-key', ['查看我的 Key', '拿到管理员发的 Key 后，查看可用模型和用量。'], ['View my key', 'Check the models and usage of a key your admin gave you.']],
  ] },
  { id: 'monitoring', group: { zh: '监控与分析', en: 'Monitoring and analytics' }, pages: [
    ['monitoring', ['渠道监控', '可用性检查和服务体检。'], ['Channel monitoring', 'Availability checks and service checkups.']],
    ['usage', ['使用分析', '查看调用量、费用、延迟和单次请求详情。'], ['Usage analytics', 'Request volume, cost, latency and per-request details.']],
  ] },
  { id: 'account', group: { zh: '帐号与安全', en: 'Account and security' }, pages: [
    ['account', ['帐号设置', '邮箱、密码、登录设备、告警邮件，以及操作日志页。'], ['Account settings', 'Email, password, signed-in devices, alert emails and the activity log.']],
    ['privacy', ['数据与隐私', 'OpenVAIR 保存什么、不保存什么。'], ['Data and privacy', 'What OpenVAIR stores and what it does not.']],
  ] },
  { id: 'help', group: { zh: '帮助', en: 'Help' }, pages: [
    ['troubleshooting', ['问题排查', '按报错和现象查找原因与解决办法。'], ['Troubleshooting', 'Find causes and fixes by error message or symptom.']],
    ['faq', ['常见问题', '部署、费用、安全和使用方式的常见疑问。'], ['FAQ', 'Common questions about deployment, cost, security and usage.']],
  ] },
]

function build(lang: DocLang) {
  const docGroups = outline.map(({ id, group, pages }) => ({
    id,
    group: group[lang],
    pages: pages.map(([slug, ...texts]): DocPage => {
      const file = `./content/${lang === 'en' ? 'en/' : ''}${slug || 'index'}.md`
      const body = sources[lang][file]
      if (body === undefined) throw new Error(`missing doc content ${file}`)
      const [title, description] = texts[lang === 'en' ? 1 : 0]
      return { slug, title, description, group: group[lang], groupId: id, body }
    }),
  }))
  return { docGroups, docPages: docGroups.flatMap(({ pages }) => pages) }
}

export const docs: Record<DocLang, ReturnType<typeof build>> = { zh: build('zh'), en: build('en') }

export const docPath = (slug: string) => slug ? `/docs/${slug}` : '/docs'
