export type Protocol = 'openai' | 'anthropic' | 'gemini'

export interface User {
  user_id: string
  email: string
  account_id: string
  role: 'owner' | 'admin' | 'member'
  is_super_admin: boolean
}

export interface Account {
  id: string
  name: string
  /** Null when unlimited, as on a self-hosted instance. */
  channel_limit: number | null
  virtual_key_limit: number | null
  api_key_prefix: string
  alert_emails: AlertEmails
}

/** Monitoring alerts emailed to the account. */
export interface AlertEmails {
  authenticity: boolean
  service: boolean
  metering: boolean
  availability: boolean
}

export interface AccountLimits {
  channel_limit: number
  virtual_key_limit: number
}

/** Site-wide capacity of homepage model verification. */
export interface PublicVerifyCapacity { workers: number; concurrency: number; max_waiting: number }

/** How rate-limited channels are routed. */
export interface RoutingSettings {
  cooldown_enabled: boolean
  max_cooldown_seconds: number
  honor_retry_after: boolean
}

export interface RegistrationSettings {
  enabled: boolean
  allowed_domains: string[]
  require_email_verification: boolean
  /** Email verification only applies once SMTP is configured. */
  smtp_configured: boolean
}

export interface AdminAccount {
  id: string
  name: string
  created_at: string
  disabled_at: string | null
  disabled_reason: string | null
  user_id: string
  email: string
  is_super_admin: boolean
  last_active_at: string | null
  channel_count: number
  key_count: number
  requests_30d: number
}

export interface HealthCheck {
  key: string
  name: string
  level: 'ok' | 'warn' | 'error'
  detail: string
  latency_ms: number | null
}

export interface SystemHealth {
  status: 'ok' | 'warn' | 'error'
  version: string
  started_at: string
  checked_at: string
  migration: { version: number; description: string } | null
  checks: HealthCheck[]
  stats: { accounts: number; disabled_accounts: number; active_sessions: number; requests_24h: number; errors_24h: number; quality_jobs_queued: number } | null
}

/** Why the upstream refused the last check: the key or the balance needs attention. */
export type HealthErrorKind = 'credential' | 'quota'
export const HEALTH_ERROR_LABELS: Record<HealthErrorKind, string> = { credential: '凭据被拒', quota: '额度不足' }

export interface Channel {
  id: string
  name: string
  // VAIR key channels accept every protocol.
  protocol: Protocol | 'any'
  base_url: string
  enabled: boolean
  priority: number
  /** Requests in flight through the channel at once; null means no cap. */
  concurrency_limit: number | null
  health_status: 'healthy' | 'unhealthy' | 'rate_limited' | 'model_unavailable' | 'unknown' | 'needs_test_model'
  health_error: string | null
  health_error_kind: HealthErrorKind | null
  /** What the last check worked around, such as a retired test model. */
  health_note: string | null
  health_checked_at: string | null
  test_model: string | null
  models: string[]
  model_mapping: Record<string, string>
  model_mode: 'allowlist' | 'passthrough'
  model_count: number
  known_models: string[]
  discovered_models: string[]
  models_synced_at: string | null
  created_at: string
  kind: 'upstream' | 'vair_key'
  /** Vendor preset the channel was created from; null for custom channels. */
  provider: string | null
  source: ChannelSource | null
}

/** Requests in flight now through a capped channel. */
export interface ChannelConcurrency {
  channel_id: string
  in_use: number
  limit: number
}

/** Vendor preset that fills a new channel's protocol and base URL. */
export interface Provider {
  id: string
  label: string
  docs_url: string
  endpoints: { protocol: Protocol; base_url: string }[]
}

/** A virtual key from another account, as its recipient may see it. */
export interface SourceKey {
  account_name: string
  key_name: string
  key_prefix: string
  allowed_models: string[]
  model_mode?: 'all' | 'allowlist'
  requests_per_minute: number | null
  token_limit_daily: number | null
  cost_limit_daily_usd: number | null
  cost_limit_total_usd: number | null
  concurrency_limit: number
  models: string[]
}

/** A key behind a share link, as previewed by the recipient. */
export type SharedKey = SourceKey & { expires_at: string }

/** Whether a source key can serve requests now; exhausted states follow the gateway's limit windows. */
export type SourceKeyStatus = 'active' | 'paused' | 'revoked' | 'exhausted_daily' | 'exhausted_total'

// A revoked key reports only its status; the other fields are absent.
export type ChannelSource = Partial<SourceKey> & {
  status: SourceKeyStatus
  used_tokens_today?: number
  used_cost_today_usd?: number
  used_cost_total_usd?: number
}

export interface MonitoringChannel {
  id: string
  name: string
  protocol: Protocol | 'any'
  kind: Channel['kind']
  enabled: boolean
  priority: number
  models: string[]
  test_model: string | null
  availability_status: 'healthy' | 'unhealthy' | 'rate_limited' | 'model_unavailable' | 'unknown'
  availability_observation: 'rate_limited' | 'model_unavailable' | 'unknown' | null
  availability_error: string | null
  availability_error_kind: HealthErrorKind | null
  availability_note: string | null
  availability_checked_at: string | null
  availability_enabled: boolean
  availability_interval_seconds: number
  /** Consecutive failures of checks and real requests; reset by any success. */
  availability_failures: number
  availability_failure_threshold: number
  availability_route_policy: 'exclude' | 'observe'
  first_token_timeout_seconds: number
}

export interface MonitoringSummary {
  total_channels: number
  available_channels: number
  channels: MonitoringChannel[]
}

export interface MonitorRun {
  id: string
  monitor_id: string | null
  channel_id: string
  kind: 'availability' | 'quality' | 'long_context'
  model: string | null
  status: 'healthy' | 'degraded' | 'unhealthy' | 'rate_limited' | 'model_unavailable' | 'unknown'
  score: number | null
  latency_ms: number | null
  ttfb_ms: number | null
  status_code: number | null
  error_code: string | null
  details: { applied?: boolean; suite_version?: string; config_version?: number; public_model?: string; upstream_model?: string; passed?: number; completed?: number; total?: number; tests?: { id: string; model?: string; passed: boolean; critical?: boolean; latency_ms: number; status_code: number | null; error_code: string | null; detail?: Record<string, unknown> | null }[]
    /** Long-context checks: the tier asked for, the largest passed and each tier run. */
    requested?: number; max_passed?: number | null; tiers?: { tokens: number; input_tokens: number | null; passed: boolean | null; latency_ms: number; error_code: string | null }[] }
  started_at: string
  finished_at: string
}

export interface MonitorRunPage {
  items: MonitorRun[]
  total: number
  page: number
  page_size: number
}

export interface QualitySettings {
  schedule_enabled: boolean
  interval_seconds: number
  timeout_ms: number
  round_timeout_ms: number
  methods: string[]
  min_score: number
  failure_threshold: number
  daily_run_limit: number
  route_policy: 'observe' | 'deprioritize' | 'exclude'
}

export interface QualityMonitor extends QualitySettings {
  id: string
  kind: 'service' | 'authenticity'
  channel_id: string
  channel_name: string
  channel_enabled: boolean
  protocol: Protocol
  model: string
  upstream_model: string
  status: 'healthy' | 'degraded' | 'unknown'
  last_observation: 'healthy' | 'degraded' | 'unknown'
  score: number | null
  consecutive_failures: number
  last_error: string | null
  checked_at: string | null
  valid_until: string | null
  next_run_at: string | null
  schedule_error: string | null
  config_version: number
  job_status: 'queued' | 'running' | 'completed' | 'failed' | 'cancelled' | null
  job_error: string | null
  identity_verdict: string | null
  identity_checked_at: string | null
  identity_evidence: { claimed_model?: string; verdict?: string; valid_samples?: number; best_model?: string | null; job_id?: string } | null
  claimed_model: string | null
  samples_per_run: number
  window_days: number
  daily_sample_limit: number
  window_verdict: string | null
  window_evidence: { claimed_model?: string; verdict?: string; valid_samples?: number; best_model?: string | null; reasons?: string[]; window?: { days: number; samples: number } } | null
  window_checked_at: string | null
  window_valid_until: string | null
  last_tests: { id: string; passed: boolean; critical?: boolean; error_code: string | null; detail?: Record<string, unknown> | null }[] | null
  long_context_max: number | null
  long_context_checked_at: string | null
}

export interface VirtualKey {
  id: string
  name: string
  /** Who the key was issued to. */
  holder: string | null
  key_prefix: string
  can_copy: boolean
  allowed_models: string[]
  model_mode: 'all' | 'allowlist'
  known_models: string[]
  requests_per_minute: number | null
  token_limit_daily: number | null
  cost_limit_daily_usd: number | null
  cost_limit_total_usd: number | null
  concurrency_limit: number
  enabled: boolean
  created_at: string
  revoked_at: string | null
  used_tokens_today: number
  used_cost_today_usd: number
  used_cost_total_usd: number
  usage_reset_daily_at: string | null
  usage_reset_total_at: string | null
  channel_mode: 'all' | 'selected'
  channel_ids: string[]
  // While set, the channel and model scope follows this group.
  group_id: string | null
  // Accounts currently using this key as a channel.
  channel_count: number
  // Of those, the accounts that accepted a share link; pasted keys stay anonymous.
  shared_accounts: { name: string; email: string | null }[]
  // Expiry of the key's open share link, if any.
  share_expires_at: string | null
}

/** A shared channel and model scope; the server copies it onto every member key. */
export interface KeyGroup {
  id: string
  name: string
  channel_mode: 'all' | 'selected'
  channel_ids: string[]
  model_mode: 'all' | 'allowlist'
  allowed_models: string[]
  // Unrevoked keys in the group.
  key_count: number
}

/** One change to a key's total cost limit; owners, holders and channel users read the same rows. */
export interface QuotaEvent {
  id: string
  kind: 'opening' | 'increase' | 'set' | 'reset'
  amount_usd: number | null
  limit_before_usd: number | null
  limit_after_usd: number | null
  used_before_usd: number | null
  note: string | null
  created_at: string
}

export interface ModelPrice {
  pattern: string
  label: string
  input_per_1m: number
  output_per_1m: number
  cached_read_per_1m: number
  synced_from: string | null
}

export interface UnpricedModel {
  model: string
  requests: number
  last_seen_at: string
}

export interface UnpricedModels {
  since: string
  models: UnpricedModel[]
}

export interface ModelCatalogEntry {
  id: string
  pattern: string
  label: string
  enabled: boolean
  created_at: string
  updated_at: string
  provider: string
  protocols: Protocol[]
  input_per_1m: number | null
  output_per_1m: number | null
  cached_read_per_1m: number | null
  price_source: string | null
}

export interface PriceRates {
  input_per_1m: number
  output_per_1m: number
  cached_read_per_1m: number
}

export interface PriceSyncChange {
  id: string
  pattern: string
  label: string
  updated_at: string
  current: PriceRates | null
  reference: PriceRates
}

export interface EmailSettings {
  configured: boolean
  host: string
  port: number
  security: 'starttls' | 'tls'
  username: string | null
  from: string
  password_configured: boolean
}

export interface UsageEvent {
  id: string
  channel_id: string | null
  channel_name: string | null
  virtual_key_id: string | null
  key_name: string | null
  key_revoked: boolean | null
  model: string
  protocol: Protocol
  status: number
  ttfb_ms: number | null
  /** When generated output first arrived; streamed requests only. */
  first_token_ms: number | null
  stream: boolean | null
  duration_ms: number
  input_tokens: number | null
  output_tokens: number | null
  cached_tokens: number | null
  created_at: string
  upstream_model: string | null
  price_snapshot: ModelPrice | null
  cost_usd: number | null
  pricing_status: 'legacy' | 'priced' | 'unpriced' | 'pending_usage'
  // 'monitor': a check another account ran through a VAIR key channel on this account's key.
  source: 'gateway' | 'monitor'
}

export type UsageRange = '24h' | '7d' | '30d' | 'all'

export type RequestOutcome = 'pending' | 'success' | 'http_error' | 'timeout' | 'first_token_timeout' | 'connection_error' | 'stream_error' | 'client_disconnected' | 'unavailable' | 'channel_full'
export interface RequestAttempt {
  channel_id: string
  channel_name: string | null
  started_ms: number
  duration_ms: number
  headers_ms: number | null
  first_token_ms: number | null
  status: number | null
  outcome: RequestOutcome
}
export interface UsageRequestDetail extends UsageEvent {
  request_diagnostics: {
    queue_ms: number | null
    attempts: RequestAttempt[]
    completion: RequestOutcome | null
  } | null
}

export interface CacheCoverage {
  cache_reported_requests: number
  cache_unreported_requests: number
  cache_unknown_requests: number
  cache_reported_input_tokens: number
  cache_reported_tokens: number
}

/** Medians over successful requests; first output and speed only from streamed ones. */
export interface UsagePerformance { p50_first_token_ms: number | null; p50_duration_ms: number | null; p50_output_tps: number | null; speed_samples: number }

export interface UsageSummary {
  since: string
  bucket_seconds: number
  totals: CacheCoverage & {
    requests: number
    successful: number
    input_tokens: number
    output_tokens: number
    cached_tokens: number
    active_models: number
    avg_duration_ms: number | null
    avg_ttfb_ms: number | null
  }
  trend: (UsagePerformance & { start: string; requests: number; tokens: number; estimated_cost_usd: number })[]
  models: { model: string; requests: number; tokens: number; estimated_cost_usd: number | null }[]
  channels: (CacheCoverage & UsagePerformance & { channel_id: string | null; channel_name: string | null; requests: number; successful: number; client_errors: number; server_errors: number; input_tokens: number; output_tokens: number; cached_tokens: number; avg_ttfb_ms: number | null; avg_duration_ms: number | null; estimated_cost_usd: number | null })[]
  model_options: string[]
  key_options: { id: string; name: string; revoked: boolean }[]
}

export interface UsageEventPage {
  items: UsageEvent[]
  total: number
  page: number
  page_size: number
}

export interface AuditEvent {
  id: string
  action: string
  resource_type: string | null
  resource_id: string | null
  metadata: Record<string, unknown>
  created_at: string
}

export interface ApiErrorBody {
  error?: { code?: string; message?: string }
}

export class ApiError extends Error {
  constructor(message: string, public status: number) { super(message); this.name = "ApiError" }
}

export async function api<T>(path: string, options: RequestInit = {}): Promise<T> {
  const headers = new Headers(options.headers)
  if (options.body && !headers.has('content-type')) headers.set('content-type', 'application/json')
  const response = await fetch(`/api${path}`, { ...options, headers, credentials: 'include' })
  const body = (await response.json().catch(() => ({}))) as T & ApiErrorBody
  if (!response.ok) {
    const message = body.error?.message ?? (response.status === 401 ? '登录已过期' : '请求失败')
    throw new ApiError(message, response.status)
  }
  return body as T
}

export function jsonBody(value: unknown, method: RequestInit['method'] = 'POST'): RequestInit {
  return { method, body: JSON.stringify(value) }
}

export function formatDate(value: string): string {
  return new Intl.DateTimeFormat('zh-CN', { dateStyle: 'medium', timeStyle: 'short' }).format(new Date(value))
}

// Compact form for tables, e.g. "10-05 10:41"; the year only appears when it is not the current one.
export function formatShortDate(value: string): string {
  const date = new Date(value)
  const pad = (part: number) => String(part).padStart(2, '0')
  const day = `${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`
  return date.getFullYear() === new Date().getFullYear() ? day : `${date.getFullYear()}-${day}`
}

/** Passive metering of one channel and upstream model over the last 24 hours of real traffic. */
export interface PassiveMetering {
  channel_id: string
  channel_name: string
  upstream_model: string
  samples: number
  tokens_per_ascii: number | null
  tokens_per_other: number | null
  overhead: number
  warnings: { kind: string }[]
  messages: string[]
  computed_at: string
  monitor_id: string | null
}
