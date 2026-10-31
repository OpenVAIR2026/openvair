import { invoke } from '@tauri-apps/api/core'

/** `codex`, `claude-code`, … — the list comes from `agents`. */
export type AgentId = string
export type Protocol = 'openai' | 'anthropic' | 'gemini'

export interface Session {
  host: string
  /** The one OpenVAIR server this computer uses; null until a key is added. */
  server: string | null
  server_host: string | null
  /** The server speaks plain HTTP, so keys travel unencrypted. */
  plain_http: boolean
  /** ... and over the internet rather than this machine's networks. */
  public_http: boolean
  key_count: number
}

/** What using an address as the server would mean for its security. */
export interface ServerCheck { server: string; plain_http: boolean; public_http: boolean }

/** A key as its server reports it (`/api/key/summary`). */
export interface KeySummary {
  id: string
  account_name: string
  key_name: string
  key_prefix: string
  status: 'active' | 'paused' | 'exhausted_daily' | 'exhausted_total' | 'cost_paused'
  requests_per_minute: number | null
  concurrency_limit: number
  token_limit_daily: number | null
  cost_limit_daily_usd: number | null
  cost_limit_total_usd: number | null
  used_tokens_today: number
  used_cost_today_usd: number
  used_cost_total_usd: number
  day_ends_at: string
  last_used_at: string | null
  models: Partial<Record<Protocol, string[]>>
  /** The address the administrator set for this server, if any. */
  server_url?: string | null
}

/** `kept` is false when every key was forgotten instead. */
export interface ServerChange { kept: boolean; server: string | null; updated: string[]; skipped: string[] }

export interface ClientKey {
  id: string
  name: string
  key_prefix: string
  enabled: boolean
  can_copy: boolean
  used_cost_today_usd: number
  cost_limit_daily_usd: number | null
  models: Partial<Record<Protocol, string[]>>
}

/** A key saved on this computer, with what its server says about it now. */
export interface SavedKey extends ClientKey {
  /** The key's name on the server; `name` may be the user's own label. */
  key_name: string
  is_default: boolean
  used_by: string[]
  summary: KeySummary | null
  /** The server rejected the key: revoked, or deleted. */
  invalid: boolean
  /** Why the summary is missing, such as an unreachable server. */
  error: string | null
}

export interface TestResult { ok: boolean; message: string; latency_ms: number; checked_at: string }

export interface Connection {
  server: string
  key_id: string
  key_name: string
  key_prefix: string
  model: string
  connected_at: string
  last_test: TestResult | null
  can_undo: boolean
  /** Claude Code's model for background work, when it was asked for. */
  background_model: string | null
  background_missing: boolean
  models_update: ModelsUpdate | null
}

/** The latest background update of an agent's model lists. */
export interface ModelsUpdate { at: string; added: string[]; removed: string[] }

export interface Agent {
  id: AgentId
  name: string
  /** Two or three characters standing in for the agent's logo. */
  glyph: string
  /** Where to learn how to install it. */
  homepage: string
  /** The protocol in use, or the default before connecting. */
  protocol: Protocol
  /** Every protocol the agent can use; the first is the default. */
  protocols: Protocol[]
  installed: boolean
  files: string[]
  connection: Connection | null
  status: 'connected' | 'edited' | 'other' | 'none'
  config_error: string | null
  notice: AgentNotice | null
  /** What to do once the file is written; null means reopening the agent. */
  after_write: string | null
}

export type AgentNotice = 'codex-chatgpt-login' | 'opencode-jsonc' | 'zcode-open-first' | 'zcode-moved'

/** Things about an agent's own setup that the app does not change. */
export const AGENT_NOTICE: Record<AgentNotice, string> = {
  'codex-chatgpt-login': 'Codex 仍以 ChatGPT 登录，每轮会先访问 chatgpt.com，连不上会超时。请确保能访问 chatgpt.com，或运行 codex logout。',
  'opencode-jsonc': 'OpenCode 的配置是 opencode.jsonc，自动接入会丢掉其中的注释。请在“接入 Agent”里选“手动接入”。',
  'zcode-open-first': '还没有找到 ZCode 的服务商配置。请先打开一次 ZCode（3.14 或更新版本），再回来接入。',
  'zcode-moved': 'ZCode 的数据目录改到了别处，OpenVAIR 不会去改。请在“接入 Agent”里选“手动接入”。',
}

/** The base URL a protocol's clients take, for tools set up by hand. */
export interface Endpoint { protocol: Protocol; base_url: string }
export interface Change { field: string; before: string | null; after: string | null }
/** `asks_background_model`: connecting repoints Claude Code's background model, so the user picks it. */
export interface Preview { changes: Change[]; asks_background_model: boolean }
/** bili context compression for Claude Code and Codex; see `core/src/bili.rs`. */
export interface BiliAgentStatus { hooked: boolean; enabled: boolean }
/** `running` is null while no agent uses bili; `error` is why the app last failed to start the proxy. */
export interface BiliStatus { npm: boolean; installed: boolean; claude_code: BiliAgentStatus; codex: BiliAgentStatus; dashboard: string | null; running: boolean | null; error: string | null }
export type BiliAgent = 'claude-code' | 'codex'
export interface AppInfo { version: string; backups_dir: string }

/** Errors from Rust arrive as `{ kind, message }`. */
export class IpcError extends Error {
  constructor(message: string, public kind: string) { super(message); this.name = 'IpcError' }
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args)
  } catch (error) {
    const value = error as { kind?: string; message?: string } | string
    if (typeof value === 'string') throw new IpcError(value, 'other')
    throw new IpcError(value.message ?? '操作失败', value.kind ?? 'other')
  }
}

export const ipc = {
  session: () => call<Session>('session'),
  keys: () => call<SavedKey[]>('keys'),
  /** `server` is only read for the first key; later keys use the saved one. */
  /** A first server on plain HTTP over the internet needs `allowPublicHttp`. */
  addKey: (server: string | null, secret: string, allowPublicHttp = false) => call<SavedKey>('add_key', { server, secret, allowPublicHttp }),
  checkServer: (server: string) => call<ServerCheck>('check_server', { server }),
  renameKey: (keyId: string, label: string) => call<void>('rename_key', { keyId, label }),
  setDefaultKey: (keyId: string) => call<void>('set_default_key', { keyId }),
  /** Refused while agents still use the key. */
  removeKey: (keyId: string) => call<void>('remove_key', { keyId }),
  /**
   * Moves to `server` keeping every key when the keys are valid there, and points connected
   * agents at it. Without `server`, or with `discardKeys`, forgets the server and every key;
   * that is refused while agents are connected.
   */
  changeServer: (server: string | null, allowPublicHttp = false, discardKeys = false) =>
    call<ServerChange>('change_server', { server, allowPublicHttp, discardKeys }),
  keySummary: (keyId: string) => call<KeySummary>('key_summary', { keyId }),
  agents: () => call<Agent[]>('agents'),
  preview: (agent: AgentId, keyId: string, model: string, models: string[], backgroundModel: string | null, protocol: Protocol) => call<Preview>('preview', { agent, keyId, model, models, backgroundModel, protocol }),
  apply: (agent: AgentId, keyId: string, model: string, backgroundModel: string | null, protocol: Protocol) => call<TestResult>('apply', { agent, keyId, model, backgroundModel, protocol }),
  test: (agent: AgentId) => call<TestResult>('test_agent', { agent }),
  disconnect: (agent: AgentId) => call<void>('disconnect', { agent }),
  undo: (agent: AgentId) => call<void>('undo', { agent }),
  biliStatus: () => call<BiliStatus>('bili_status'),
  openBili: () => call<void>('open_bili'),
  installBili: () => call<BiliStatus>('install_bili'),
  enableBili: (agent: BiliAgent) => call<BiliStatus>('enable_bili', { agent }),
  disableBili: (agent: BiliAgent) => call<BiliStatus>('disable_bili', { agent }),
  autostart: () => call<boolean>('autostart'),
  setAutostart: (on: boolean) => call<boolean>('set_autostart', { on }),
  syncSetting: () => call<boolean>('sync_setting'),
  setSyncSetting: (on: boolean) => call<boolean>('set_sync_setting', { on }),
  refreshModels: () => call<string[]>('refresh_models'),
  appInfo: () => call<AppInfo>('app_info'),
  openBackups: () => call<void>('open_backups'),
  /** Opens a page of the server, such as `/my-key/usage`. */
  openConsole: (path: string) => call<void>('open_console', { path }),
  openExternal: (url: string) => call<void>('open_external', { url }),
  /** Base URLs on the server. */
  endpoints: () => call<Endpoint[]>('endpoints'),
  copyText: (text: string) => call<void>('copy_text', { text }),
  /** The secret goes to the clipboard from Rust and never reaches the window. */
  copyKey: (keyId: string) => call<void>('copy_key', { keyId }),
}

/** Where new versions are published; the open-source build has no updater. */
export const RELEASES_URL = 'https://github.com/OpenVAIR2026/openvair/releases'

export const PROTOCOL_LABEL: Record<Protocol, string> = { openai: 'OpenAI', anthropic: 'Anthropic', gemini: 'Gemini' }

export function relativeTime(value: string) {
  const minutes = Math.round((Date.now() - new Date(value).getTime()) / 60000)
  if (minutes < 1) return '刚刚'
  if (minutes < 60) return `${minutes} 分钟前`
  const hours = Math.round(minutes / 60)
  if (hours < 24) return `${hours} 小时前`
  return `${Math.round(hours / 24)} 天前`
}

export function daysUntil(value: string) {
  return Math.max(0, Math.ceil((new Date(value).getTime() - Date.now()) / 86400000))
}

export const money = (value: number) => `$${value.toFixed(2)}`

/** Why a key cannot serve requests now, and what fixes it. */
export const HOLDER_STATUS: Record<KeySummary['status'], { label: string; ok: boolean; detail: string }> = {
  active: { label: '正常', ok: true, detail: '这把 Key 可以正常调用。' },
  paused: { label: '已暂停', ok: false, detail: '管理员暂停了这把 Key，请联系管理员恢复。' },
  exhausted_daily: { label: '今日已用完', ok: false, detail: '今日额度已用完，次日自动恢复，也可以请管理员重置。' },
  exhausted_total: { label: '额度已用完', ok: false, detail: '总费用额度已用完，请管理员提高上限或重置。' },
  cost_paused: { label: '已暂停', ok: false, detail: '今日有请求的费用无法确定，按管理员设置暂停到次日。' },
}
