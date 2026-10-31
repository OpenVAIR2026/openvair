// Dev-only stand-in for the Rust side, so `npm run dev` renders in a plain
// browser. Never bundled into the app: main.tsx imports it only in dev
// builds outside Tauri.
import type { Agent, BiliStatus, KeySummary, SavedKey, TestResult } from './ipc'

const now = () => new Date().toISOString()
const ok: TestResult = { ok: true, message: '连接正常', latency_ms: 212, checked_at: new Date(Date.now() - 120000).toISOString() }
const models = { openai: ['gpt-5.6', 'gpt-5.6-sol'], anthropic: ['claude-sonnet-5-5'], gemini: ['gemini-2.5-pro'] }

// ?state=welcome starts with no key, ?state=paused has the default key paused,
// ?state=lan uses a plain-HTTP server on the local network, ?state=public-http one on the internet,
// ?state=moved has the administrator announcing a new address.
const state = new URLSearchParams(location.search).get('state')
const summary = (id: string, key_name: string, key_prefix: string, extra: Partial<KeySummary> = {}): KeySummary => ({
  id, account_name: 'OpenVAIR', key_name, key_prefix, status: 'active',
  requests_per_minute: 30, concurrency_limit: 2, token_limit_daily: 200000, cost_limit_daily_usd: 5, cost_limit_total_usd: 50,
  used_tokens_today: 12840, used_cost_today_usd: 0.42, used_cost_total_usd: 12.4, day_ends_at: new Date(Date.now() + 6 * 3600000).toISOString(),
  last_used_at: now(), models, server_url: state === 'moved' ? 'https://vair.example.com' : null, ...extra,
})
const saved = (id: string, name: string, key_prefix: string, extra: Partial<SavedKey> = {}): SavedKey => {
  const info = extra.summary === undefined ? summary(id, name, key_prefix) : extra.summary
  return {
    id, name, key_prefix, enabled: info?.status !== 'paused', can_copy: true,
    used_cost_today_usd: info?.used_cost_today_usd ?? 0, cost_limit_daily_usd: info?.cost_limit_daily_usd ?? null, models: info?.models ?? {},
    key_name: name, is_default: false,
    used_by: [], summary: info, invalid: false, error: null, ...extra,
  }
}
let keys: SavedKey[] = state === 'welcome' ? [] : [
  saved('k1', 'MacBook · Codex', 'openvair_Ab3x', { is_default: true, used_by: ['Codex'], summary: summary('k1', 'MacBook · Codex', 'openvair_Ab3x', { status: state === 'paused' ? 'paused' : 'active' }) }),
  saved('k3', 'MacBook · Claude', 'openvair_K9xa', { used_by: ['Claude Code'], summary: summary('k3', 'MacBook · Claude', 'openvair_K9xa', { cost_limit_daily_usd: 3, used_cost_today_usd: 0.18 }) }),
  saved('k4', 'DeepSeek', 'openvair_Ds4p', { summary: summary('k4', 'DeepSeek', 'openvair_Ds4p', { models: { anthropic: ['deepseek-v4-flash', 'deepseek-v4-pro'], openai: ['deepseek-v4-flash', 'deepseek-v4-pro'] } }) }),
  saved('k2', '旧 Key', 'openvair_Qm8r', { summary: null, invalid: true }),
]
// ?docs drops the revoked key too, so the screenshots show no warnings.
if (new URLSearchParams(location.search).has('docs')) keys = keys.filter(key => !key.invalid)
let server: string | null = keys.length ? (state === 'lan' ? 'http://192.168.1.20:8080/' : state === 'public-http' ? 'http://203.0.113.7:8080/' : 'https://openvair.example.com/') : null
const agents: Agent[] = [
  { id: 'codex', name: 'Codex', after_write: null, glyph: '>_', homepage: 'https://github.com/openai/codex', protocol: 'openai', protocols: ['openai'], installed: true, files: ['~/.codex/config.toml'], status: 'connected', config_error: null, notice: 'codex-chatgpt-login',
    connection: { server: 'https://openvair.example.com/', key_id: 'k1', key_name: 'MacBook · Codex', key_prefix: 'openvair_Ab3x', model: 'gpt-5.6', connected_at: now(), last_test: ok, can_undo: true, background_model: null, background_missing: false, models_update: { at: now(), added: ['deepseek-v4.1-flash'], removed: [] } } },
  { id: 'claude-code', name: 'Claude Code', after_write: null, glyph: 'CC', homepage: 'https://docs.anthropic.com/claude-code', protocol: 'anthropic', protocols: ['anthropic'], installed: true, files: ['~/.claude/settings.json'], status: 'connected', config_error: null, notice: null,
    connection: { server: 'https://openvair.example.com/', key_id: 'k3', key_name: 'MacBook · Claude', key_prefix: 'openvair_K9xa', model: 'claude-sonnet-5-5', connected_at: now(), last_test: ok, can_undo: true, background_model: null, background_missing: false, models_update: null } },
  { id: 'gemini-cli', name: 'Gemini CLI', after_write: null, glyph: 'G', homepage: 'https://github.com/google-gemini/gemini-cli', protocol: 'gemini', protocols: ['gemini'], installed: true, files: ['~/.gemini/.env', '~/.gemini/settings.json'], status: 'none', config_error: null, notice: null, connection: null },
  { id: 'qwen-code', name: 'Qwen Code', after_write: null, glyph: 'Q', homepage: 'https://github.com/QwenLM/qwen-code', protocol: 'openai', protocols: ['openai', 'anthropic'], installed: true, files: ['~/.qwen/settings.json'], status: 'none', config_error: null, notice: null, connection: null },
  { id: 'kimi-cli', name: 'Kimi CLI', after_write: null, glyph: 'K', homepage: 'https://github.com/MoonshotAI/kimi-cli', protocol: 'openai', protocols: ['openai', 'anthropic'], installed: false, files: ['~/.kimi/config.toml'], status: 'none', config_error: null, notice: null, connection: null },
  { id: 'opencode', name: 'OpenCode', after_write: null, glyph: 'OC', homepage: 'https://opencode.ai/docs/', protocol: 'openai', protocols: ['openai', 'anthropic'], installed: true, files: ['~/.config/opencode/opencode.json'], status: 'none', config_error: null, notice: null, connection: null },
  { id: 'zcode', name: 'ZCode', after_write: 'ZCode 会自动读取新配置。首次打开如果要求登录，可选“使用 API key”，再点“暂时跳过”。', glyph: 'Z', homepage: 'https://zcode.z.ai/', protocol: 'openai', protocols: ['openai', 'anthropic'], installed: true, files: ['~/.zcode/v2/provider_config.json'], status: 'none', config_error: null, notice: 'zcode-open-first', connection: null },
  { id: 'dsh', name: 'DeepSeek Harness', after_write: '重新启动 dsh 后生效。OpenVAIR 的设置写在 ~/.dsh/cordis.patch.yml，优先于 dsh 设置页里的 DeepSeek 模型配置，断开后恢复。', glyph: 'DS', homepage: 'https://www.npmjs.com/package/@deepseek-ai/dsh', protocol: 'anthropic', protocols: ['anthropic'], installed: false, files: ['~/.dsh/cordis.patch.yml', '~/.dsh/.env'], status: 'none', config_error: null, notice: null, connection: null },
]
// ?docs shows a healthy page for the documentation screenshots: no warnings.
const docs = new URLSearchParams(location.search).has('docs')
if (docs) {
  agents[0] = { ...agents[0], notice: null, connection: { ...agents[0].connection!, models_update: null } }
  agents[1] = { ...agents[1], connection: { ...agents[1].connection!, key_id: 'k3', key_name: 'MacBook · Claude', key_prefix: 'openvair_K9xa' } }
}
// ?agents=many connects six, enough for the wheel; ?agents=home is the same six with
// DeepSeek Harness right after Claude Code, for the homepage screenshot.
const agentsParam = new URLSearchParams(location.search).get('agents')
if (agentsParam === 'many' || agentsParam === 'home') {
  const link = (id: string, key_name: string, model: string) => {
    const index = agents.findIndex(agent => agent.id === id)
    agents[index] = { ...agents[index], installed: true, status: 'connected', notice: null, connection: { ...agents[0].connection!, key_id: 'k3', key_name, key_prefix: 'openvair_K9xa', model, models_update: null } }
  }
  link('gemini-cli', 'MacBook · Claude', 'gemini-2.5-pro')
  link('qwen-code', 'DeepSeek', 'deepseek-v4-pro')
  link('opencode', 'MacBook · Claude', 'gpt-5.6-sol')
  link('dsh', 'DeepSeek', 'deepseek-v4-flash')
  if (agentsParam === 'home') agents.splice(2, 0, ...agents.splice(agents.findIndex(agent => agent.id === 'dsh'), 1))
}
// ?agents=few has Codex and Claude Code connected and Gemini CLI installed, as on most computers.
if (agentsParam === 'few') agents.forEach((agent, index) => { if (index > 2) agents[index] = { ...agent, installed: false } })
// ?bili=none has no Node.js, ?bili=installed has bili, ?bili=on is compressing.
const biliParam = new URLSearchParams(location.search).get('bili')
const biliOn = { hooked: biliParam === 'on', enabled: biliParam === 'on' }
let bili: BiliStatus = { npm: biliParam !== 'none', installed: biliParam === 'on' || biliParam === 'installed', claude_code: biliOn, codex: biliOn, dashboard: biliParam === 'on' ? 'http://127.0.0.1:18787' : null, running: biliParam === 'on' ? true : null, error: null }
let autostart = false
let syncModels = true
const biliAgent = (args: Record<string, unknown>) => args.agent === 'codex' ? 'codex' : 'claude_code'
const biliDashboard = (next: BiliStatus) => next.claude_code.enabled || next.codex.enabled ? 'http://127.0.0.1:18787' : null
const biliRouted = (next: BiliStatus): BiliStatus => { const dashboard = biliDashboard(next); return { ...next, dashboard, running: dashboard ? true : null } }

// A rough stand-in for the Rust check: plain HTTP to anything but localhost, LAN or tailnet addresses.
const isPublicHttp = (address: string) => {
  const url = new URL(address.includes('://') ? address : `https://${address}`)
  return url.protocol === 'http:' && !/^(localhost|[^.]+|.*\.(localhost|local|lan|internal|home\.arpa)|10\..*|192\.168\..*|172\.(1[6-9]|2\d|3[01])\..*|100\.(6[4-9]|[7-9]\d|1[01]\d|12[0-7])\..*|127\..*|\[.*\])$/.test(url.hostname)
}

const handlers: Record<string, (args: Record<string, unknown>) => unknown> = {
  session: () => ({ host: 'MacBook-Pro', server, server_host: server && new URL(server).host, plain_http: !!server?.startsWith('http:'), public_http: !!server && isPublicHttp(server), key_count: keys.length }),
  keys: () => keys,
  check_server: args => {
    const address = String(args.server)
    const url = new URL(address.includes('://') ? address : `https://${address}`)
    return { server: url.origin + '/', plain_http: url.protocol === 'http:', public_http: isPublicHttp(address) }
  },
  add_key: args => {
    if (!server && isPublicHttp(String(args.server)) && !args.allowPublicHttp) throw { kind: 'other', message: '公网地址使用 http:// 时 Key 会明文传输，请先确认' }
    const key = saved(`k${Date.now()}`, '新 Key', 'openvair_N3wk', { is_default: keys.length === 0 })
    server ??= String(args.server)
    keys = [...keys, key]
    return key
  },
  rename_key: args => { keys = keys.map(key => key.id === args.keyId ? { ...key, name: String(args.label) || key.key_name } : key); return null },
  set_default_key: args => { keys = keys.map(key => ({ ...key, is_default: key.id === args.keyId })); return null },
  remove_key: args => { keys = keys.filter(key => key.id !== args.keyId); if (!keys.length) server = null; return null },
  change_server: args => {
    const connected = agents.filter(agent => agent.connection).map(agent => agent.name)
    if (args.server && !args.discardKeys) {
      const next = new URL(String(args.server)).origin + '/'
      if (next === server) throw { kind: 'other', message: '新地址和当前地址相同' }
      if (isPublicHttp(next) && !args.allowPublicHttp) throw { kind: 'other', message: '公网地址使用 http:// 时 Key 会明文传输，请先确认' }
      if (!next.includes('example')) throw { kind: 'other', message: `「${keys[0]?.name}」在 ${new URL(next).host} 上无效，那里不是同一台服务器，Key 无法保留` }
      server = next
      keys = keys.map(key => key.summary ? { ...key, summary: { ...key.summary, server_url: null } } : key)
      return { kept: true, server, updated: connected, skipped: [] }
    }
    if (connected.length) throw { kind: 'other', message: `请先断开 ${connected.join('、')}，再更换服务器` }
    keys = []
    server = null
    return { kept: false, server: null, updated: [], skipped: [] }
  },
  key_summary: args => keys.find(key => key.id === args.keyId)?.summary,
  agents: () => agents,
  preview: args => {
    const models = (args.models ?? []) as string[]
    const asks = models.length > 0 && !models.some(name => /claude|anthropic/i.test(name))
    const others = [...new Set([...models, String(args.model)])].filter(name => !/claude|anthropic/i.test(name)).sort()
    return {
      asks_background_model: asks,
      changes: [
        { field: 'env.ANTHROPIC_BASE_URL', before: 'https://openvair.example.com', after: 'https://openvair.example.com' },
        { field: 'env.ANTHROPIC_AUTH_TOKEN', before: 'openvair_Qm8r…••••', after: `${keys.find(key => key.id === args.keyId)?.key_prefix ?? 'vair_'}…••••` },
        { field: 'env.ANTHROPIC_MODEL', before: 'claude-sonnet-5-5', after: args.model },
        ...(asks ? [
          { field: 'env.ANTHROPIC_DEFAULT_HAIKU_MODEL', before: null, after: args.backgroundModel ?? args.model },
          { field: 'env.ANTHROPIC_DEFAULT_SONNET_MODEL', before: null, after: args.model },
          { field: 'env.ANTHROPIC_DEFAULT_OPUS_MODEL', before: null, after: args.model },
        ] : []),
        ...(others.length ? [{ field: 'modelPicker（/model 菜单）', before: null, after: others.join(', ') }] : []),
      ],
    }
  },
  apply: () => ok,
  endpoints: () => [{ protocol: 'openai', base_url: 'https://openvair.example.com/v1' }, { protocol: 'anthropic', base_url: 'https://openvair.example.com' }, { protocol: 'gemini', base_url: 'https://openvair.example.com' }],
  copy_text: () => undefined,
  copy_key: () => undefined,
  test_agent: () => ok,
  disconnect: () => null,
  undo: () => null,
  bili_status: () => bili,
  install_bili: () => { bili = { ...bili, npm: true, installed: true }; return bili },
  enable_bili: args => { bili = biliRouted({ ...bili, [biliAgent(args)]: { hooked: true, enabled: true } }); return bili },
  disable_bili: args => { bili = biliRouted({ ...bili, [biliAgent(args)]: { hooked: false, enabled: false } }); return bili },
  autostart: () => autostart,
  set_autostart: args => { autostart = !!args.on; return autostart },
  sync_setting: () => syncModels,
  set_sync_setting: args => { syncModels = !!args.on; return syncModels },
  refresh_models: () => ['Codex'],
  open_bili: () => null,
  app_info: () => ({ version: '0.1.0', backups_dir: '~/Library/Application Support/OpenVAIR/backups' }),
  open_backups: () => null,
  open_console: () => null,
  open_external: () => null,
}

export function installMock() {
  ;(window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
    invoke: async (command: string, args: Record<string, unknown>) => {
      await new Promise(resolve => setTimeout(resolve, 150))
      return handlers[command]?.(args ?? {}) ?? null
    },
    transformCallback: () => 0,
  }
}
