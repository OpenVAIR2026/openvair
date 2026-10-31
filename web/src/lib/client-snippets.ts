import { CLOUD, PRODUCT } from './edition'

const PROVIDER = CLOUD ? 'vair' : 'openvair'
const KEY_ENV = CLOUD ? 'VAIR_API_KEY' : 'OPENVAIR_API_KEY'

export type ClientId = 'claude-code' | 'codex' | 'gemini-cli' | 'cc-switch' | 'generic' | 'curl'

export interface ClientSnippet {
  id: ClientId
  name: string
  label: string
  code: string
  note: string
  /** Model names this client sends by default; used to warn when the key cannot route them. */
  expects?: { prefix: string; hint: string }
}

export interface SnippetValues {
  origin: string
  key?: string
  model?: string
}

export const KEY_PLACEHOLDER = `你的 ${PRODUCT} Key`
const keyPlaceholder = KEY_PLACEHOLDER
const modelPlaceholder = '已开放的模型名'

/** Client setup snippets shared by the homepage and the console. Values default to readable placeholders. */
export function clientSnippets({ origin, key = keyPlaceholder, model }: SnippetValues, { curl = false } = {}): ClientSnippet[] {
  // Claude Code sends claude-* names unless told otherwise; only a Claude model is a safe guess for the example.
  const anthropicModel = model?.startsWith('claude-') ? model : 'Anthropic 协议的模型名'
  const snippets: ClientSnippet[] = [
    {
      id: 'claude-code',
      name: 'Claude Code',
      label: '终端环境变量',
      code: [
        `export ANTHROPIC_BASE_URL="${origin}"`,
        `export ANTHROPIC_AUTH_TOKEN="${key}"`,
        ...['MODEL', 'DEFAULT_OPUS_MODEL', 'DEFAULT_SONNET_MODEL', 'DEFAULT_HAIKU_MODEL'].map(name => `export ANTHROPIC_${name}="${anthropicModel}"`),
      ].join('\n'),
      note: '设置后在同一个终端里照常启动 Claude Code。模型可以是这把 Key 里任意 Anthropic 协议的模型，不必是 Claude；标题生成等后台任务也会用它。',
      expects: { prefix: 'claude-', hint: '这把 Key 当前没有 Claude 模型。请把示例里的模型名换成这把 Key 里 Anthropic 协议的模型。' },
    },
    {
      id: 'codex',
      name: 'Codex',
      label: '~/.codex/config.toml',
      code: [
        `model = "${model ?? modelPlaceholder}"`,
        `model_provider = "${PROVIDER}"`,
        '',
        `[model_providers.${PROVIDER}]`,
        `name = "${PRODUCT}"`,
        `base_url = "${origin}/v1"`,
        `env_key = "${KEY_ENV}"`,
        'wire_api = "responses"',
      ].join('\n'),
      note: `再把环境变量 ${KEY_ENV} 设为${key === keyPlaceholder ? `你的 ${PRODUCT} Key` : `这把 ${PRODUCT} Key`}，之后照常启动 Codex。`,
    },
    {
      id: 'gemini-cli',
      name: 'Gemini CLI',
      label: '终端环境变量',
      code: [
        `export GOOGLE_GEMINI_BASE_URL="${origin}"`,
        `export GEMINI_API_KEY="${key}"`,
      ].join('\n'),
      note: '设置后在同一个终端里照常启动 Gemini CLI，首次登录时选择 Gemini API Key。模型名需在后台已开放。',
      expects: { prefix: 'gemini-', hint: 'Gemini CLI 默认请求 gemini-* 模型，这把 Key 当前没有可用的 Gemini 模型。请先在渠道中开放同名模型，或为这把 Key 放开相应模型。' },
    },
    {
      id: 'cc-switch',
      name: 'CC Switch',
      label: '添加自定义供应商',
      code: [
        `供应商名称：${PRODUCT}`,
        `API Key：${key}`,
        `请求地址：${origin}`,
      ].join('\n'),
      note: `在 CC Switch 的 Claude、Codex 或 Gemini 页面添加自定义供应商，填写后切换到 ${PRODUCT}。Codex 的请求地址末尾需加 /v1。`,
    },
    {
      id: 'generic',
      name: '其他客户端',
      label: '设置中的自定义接口',
      code: [
        `接口地址：${origin}/v1`,
        `API Key：${key}`,
        `模型：${model ?? '后台已开放的模型名'}`,
      ].join('\n'),
      note: '适用于支持自定义接口地址的客户端。有的客户端不需要末尾的 /v1，以客户端说明为准。',
    },
  ]
  if (curl) snippets.push({
    id: 'curl',
    name: 'curl',
    label: '终端',
    code: [
      `curl ${origin}/v1/chat/completions \\`,
      `  -H "Authorization: Bearer ${key}" \\`,
      '  -H "Content-Type: application/json" \\',
      `  -d '{"model":"${model ?? modelPlaceholder}","messages":[{"role":"user","content":"Hello"}]}'`,
    ].join('\n'),
    note: '适用于 OpenAI 兼容模型。Anthropic 或 Gemini 协议的模型请改用对应客户端测试。',
  })
  return snippets
}

/** A model to show in examples: prefer one the client is built for, else the first routable one. */
export function exampleModel(models: string[], prefix?: string) {
  return (prefix ? models.find(model => model.startsWith(prefix)) : undefined) ?? models[0]
}
