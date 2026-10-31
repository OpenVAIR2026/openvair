import { Fragment, useEffect, useRef, useState, type FormEvent, type ReactNode } from 'react'
import { keepPreviousData, useQuery } from '@tanstack/react-query'
import { Link, NavLink, useLocation } from 'react-router-dom'
import { Bar, BarChart, CartesianGrid, ResponsiveContainer, Tooltip, XAxis, YAxis } from 'recharts'
import { Activity, ChartColumn, Check, CircleAlert, CircleCheck, Clock3, Code2, Coins, Copy, Download, KeyRound, LayoutDashboard, LogOut, RefreshCw, Zap } from 'lucide-react'
import { ApiError, formatDate, formatShortDate, type ApiErrorBody, type Protocol, type QuotaEvent, type SourceKeyStatus, type UsageRange } from '../lib/api'
import { copyText } from '../lib/clipboard'
import { clientSnippets, exampleModel, type ClientId } from '../lib/client-snippets'
import { usageRanges } from '../lib/overview'
import { queryClient, useOrigin } from '../lib/queries'
import { CLOUD, PRODUCT } from '../lib/edition'
import { usageAxisTick, usageCost } from '../lib/usage-metrics'
import { Badge, Choice, Empty, ErrorBox, Field, Loading, Panel, StatCard, StatGrid } from '../components/common'
import { BrandMark } from '../components/brand'
import { SiteHeader } from '../components/site-header'
import { Button } from '../components/ui/button'
import { Checkbox } from '../components/ui/checkbox'
import { Input } from '../components/ui/input'
import { KeyRequestDetails, type KeyRequestDetail } from '../components/key-request-details'
import { QuotaHistory } from '../components/quota-history'

interface KeySummary {
  account_name: string
  key_name: string
  key_prefix: string
  status: Exclude<SourceKeyStatus, 'revoked'>
  requests_per_minute: number | null
  concurrency_limit: number
  token_limit_daily: number | null
  cost_limit_daily_usd: number | null
  cost_limit_total_usd: number | null
  used_tokens_today: number
  used_cost_today_usd: number
  used_cost_total_usd: number
  day_ends_at: string
  created_at: string
  last_used_at: string | null
  models: Partial<Record<Protocol, string[]>>
}

interface KeyUsage {
  bucket_seconds: number
  totals: { requests: number; successful: number; input_tokens: number; output_tokens: number; cached_tokens: number; known_cost_usd: number; avg_duration_ms: number | null }
  trend: { start: string; requests: number; tokens: number; estimated_cost_usd: number }[]
  models: { model: string; requests: number; tokens: number; estimated_cost_usd: number | null }[]
}

interface KeyEvent {
  id: string
  created_at: string
  model: string
  protocol: Protocol
  status: number
  ttfb_ms: number | null
  first_token_ms: number | null
  stream: boolean | null
  duration_ms: number
  input_tokens: number | null
  output_tokens: number | null
  cached_tokens: number | null
  cost_usd: number | null
  pricing_status: string
  source: 'gateway' | 'monitor'
}

interface KeyEvents { items: KeyEvent[]; total: number; page: number; page_size: number }

// The key lives only in this tab unless the holder asks this browser to remember it.
const STORAGE = 'vair.holder-key'
const PAGE_SIZE = 10
// Gateway errors link to the overview, so it keeps the bare path.
const PAGE_PATH = '/my-key'
const USAGE_PATH = '/my-key/usage'
const protocolLabels: Record<Protocol, string> = { openai: 'OpenAI', anthropic: 'Anthropic', gemini: 'Gemini' }
const rangeOptions: { value: UsageRange; label: string }[] = [...usageRanges, { value: 'all', label: '全部时间' }]
const modelPrefix: Partial<Record<ClientId, string>> = { 'claude-code': 'claude-', 'gemini-cli': 'gemini-', codex: 'gpt-', generic: 'gpt-', curl: 'gpt-' }
// Marks where the key goes in a snippet, so the page never prints it in full.
const keyToken = '\u0000vair-key\u0000'

function readStored() {
  for (const storage of [() => sessionStorage, () => localStorage]) {
    try { const value = storage().getItem(STORAGE); if (value) return value } catch { /* storage blocked */ }
  }
  return null
}

function store(value: string | null, remember = false) {
  for (const [storage, keep] of [[() => sessionStorage, !remember], [() => localStorage, remember]] as const) {
    try { if (value && keep) storage().setItem(STORAGE, value); else storage().removeItem(STORAGE) } catch { /* storage blocked: keep it in memory only */ }
  }
}

/** Calls a key-holder endpoint with the key itself as the credential; no cookies are sent. */
async function holderApi<T>(path: string, key: string, signal?: AbortSignal): Promise<T> {
  const response = await fetch(`/api/key${path}`, { headers: { authorization: `Bearer ${key}` }, credentials: 'omit', cache: 'no-store', signal })
  const body = await response.json().catch(() => ({})) as T & ApiErrorBody
  if (!response.ok) throw new ApiError(body.error?.message ?? '请求失败', response.status)
  return body
}

const whole = (value: number) => new Intl.NumberFormat('zh-CN').format(value)
const compact = (value: number) => new Intl.NumberFormat('zh-CN', { notation: 'compact', maximumFractionDigits: 1 }).format(value)
const duration = (value: number) => value >= 1000 ? `${(value / 1000).toFixed(1)}s` : `${whole(Math.round(value))}ms`
const money = (value: number) => `$${value.toFixed(2)}`

/** Self-service page for people who were given only a key. */
export function MyKeyPage() {
  const [key, setKey] = useState(readStored)
  const [notice, setNotice] = useState<string | null>(null)
  function signOut(message: string | null = null) {
    store(null)
    queryClient.removeQueries({ queryKey: ['key-holder'] })
    setKey(null)
    setNotice(message)
  }
  return <main className="min-h-dvh">
    <SiteHeader actions={key ? <Button variant="ghost" size="sm" onClick={() => signOut()}><LogOut aria-hidden="true" />清除 Key</Button> : undefined} />
    {key
      ? <Dashboard secret={key} onInvalid={message => signOut(message)} />
      : <EnterKey notice={notice} onSubmit={(value, remember) => { store(value, remember); setNotice(null); setKey(value) }} />}
  </main>
}

function EnterKey({ notice, onSubmit }: { notice: string | null; onSubmit: (key: string, remember: boolean) => void }) {
  const [value, setValue] = useState('')
  const [remember, setRemember] = useState(false)
  const [checking, setChecking] = useState(false)
  const [error, setError] = useState<Error | null>(null)
  async function submit(event: FormEvent) {
    event.preventDefault()
    const key = value.trim()
    if (!key) return
    setChecking(true); setError(null)
    try {
      const summary = await holderApi<KeySummary>('/summary', key)
      queryClient.setQueryData(['key-holder', 'summary', key.slice(0, 12)], summary)
      onSubmit(key, remember)
    } catch (e) {
      setError(e as Error)
    } finally {
      setChecking(false)
    }
  }
  return <div className="mx-auto grid max-w-lg gap-6 px-4 py-12">
    <Panel className="p-6 sm:p-8">
      <form className="grid gap-5" onSubmit={submit}>
        <div>
          <h1 className="text-[24px] font-semibold tracking-tight">查看我的 {PRODUCT} Key</h1>
          <p className="mt-2 text-sm leading-6 text-muted-foreground">粘贴别人发给你的 {PRODUCT} Key，查看它的状态、额度、可用模型和调用记录，以及各客户端的接入方式。</p>
        </div>
        {notice && <p role="status" className="text-sm text-warning">{notice}</p>}
        <Field label={`${PRODUCT} Key`}>{id => <Input id={id} type="password" value={value} onChange={e => { setValue(e.target.value); setError(null) }} autoComplete="off" spellCheck={false} placeholder={CLOUD ? 'vair_…' : 'openvair_…'} className="font-mono" />}</Field>
        <label className="flex items-start gap-2 text-sm"><Checkbox checked={remember} onCheckedChange={next => setRemember(next === true)} className="mt-0.5" /><span>在此浏览器记住这把 Key<span className="block text-xs leading-5 text-muted-foreground">不勾选时，关闭标签页后需要重新粘贴。公共电脑上请不要勾选。</span></span></label>
        <ErrorBox error={error} />
        <Button type="submit" className="h-11" disabled={checking || !value.trim()}>{checking ? '验证中…' : '查看'}</Button>
      </form>
    </Panel>
    <p className="text-center text-xs leading-5 text-muted-foreground">这里只能查看，不能修改 Key。有 {PRODUCT} 帐号？<Link to="/login" className="text-primary hover:underline">登录控制台</Link></p>
  </div>
}

function Dashboard({ secret, onInvalid }: { secret: string; onInvalid: (message: string) => void }) {
  const id = secret.slice(0, 12)
  const summary = useQuery({
    queryKey: ['key-holder', 'summary', id],
    queryFn: ({ signal }) => holderApi<KeySummary>('/summary', secret, signal),
    retry: false,
    refetchInterval: 60_000,
  })
  const usageTab = useLocation().pathname === USAGE_PATH
  // A key revoked while the page is open sends the holder back to the form.
  const rejected = summary.error instanceof ApiError && summary.error.status === 401 ? summary.error.message : null
  useEffect(() => { if (rejected) onInvalid(rejected) }, [rejected, onInvalid])
  if (rejected) return null
  return <div className="mx-auto grid max-w-6xl gap-5 px-4 py-6 md:px-8 md:py-8">
    <nav aria-label="我的 Key" className="flex flex-wrap items-center gap-3">
      <div className="grid w-full grid-cols-2 gap-1 rounded-xl border border-border bg-card p-1 sm:inline-grid sm:w-auto">
        <TabLink to={PAGE_PATH} icon={<LayoutDashboard aria-hidden="true" className="size-4" />}>概览</TabLink>
        <TabLink to={USAGE_PATH} icon={<ChartColumn aria-hidden="true" className="size-4" />}>用量</TabLink>
      </div>
      {/* The usage tab has no status card, so name the key it is showing. */}
      {usageTab && summary.data && <span className="flex min-w-0 items-center gap-2 text-sm text-muted-foreground sm:ml-auto"><span className="truncate">{summary.data.key_name}</span><Badge tone={statusCopy[summary.data.status].tone}><span className="size-1.5 rounded-full bg-current" />{statusCopy[summary.data.status].label}</Badge></span>}
    </nav>
    {summary.isPending ? <Loading /> : summary.isError ? <ErrorBox error={summary.error} retry={() => summary.refetch()} /> : <>
      {usageTab ? <UsagePanel secret={secret} id={id} /> : <>
        <StatusPanel summary={summary.data} refreshing={summary.isFetching} onRefresh={() => queryClient.invalidateQueries({ queryKey: ['key-holder'] })} />
        <div className="grid gap-5 lg:grid-cols-[minmax(0,2fr)_minmax(0,3fr)]">
          <ModelsPanel models={summary.data.models} />
          <ConnectPanel secret={secret} models={summary.data.models} />
        </div>
        <QuotaPanel secret={secret} id={id} capped={summary.data.cost_limit_total_usd != null} />
      </>}
      {CLOUD && <p className="text-center text-xs leading-5 text-muted-foreground">想同时使用自己的供应商 Key，或管理多把 Key？<Link to="/login" className="text-primary hover:underline">注册 VAIR 帐号</Link>后，可以把这把 Key 添加为渠道。</p>}
    </>}
  </div>
}

function TabLink({ to, icon, children }: { to: string; icon: ReactNode; children: ReactNode }) {
  return <NavLink to={to} end className={({ isActive }) => `inline-flex items-center justify-center gap-2 rounded-lg px-5 py-2 text-sm font-medium transition-colors focus-visible:outline-2 focus-visible:outline-ring ${isActive ? 'bg-primary text-primary-foreground shadow-sm' : 'text-muted-foreground hover:bg-muted/60 hover:text-foreground'}`}>{icon}{children}</NavLink>
}

const statusCopy: Record<KeySummary['status'], { label: string; tone: 'success' | 'warning' | 'danger'; detail: (s: KeySummary) => string }> = {
  active: { label: '正常', tone: 'success', detail: () => '这把 Key 可以正常调用。' },
  paused: { label: '已暂停', tone: 'warning', detail: s => `${s.account_name} 暂停了这把 Key，请联系发放方恢复。` },
  exhausted_daily: { label: '今日已用完', tone: 'warning', detail: s => `今日额度已用完，${formatShortDate(s.day_ends_at)} 自动恢复，也可以请发放方重置。` },
  exhausted_total: { label: '额度已用完', tone: 'danger', detail: () => '总费用额度已用完，请发放方提高上限或重置。' },
}

function StatusPanel({ summary, refreshing, onRefresh }: { summary: KeySummary; refreshing: boolean; onRefresh: () => void }) {
  const status = statusCopy[summary.status]
  const Icon = status.tone === 'success' ? CircleCheck : CircleAlert
  const quotas = [
    { label: '今日 Token', used: summary.used_tokens_today, limit: summary.token_limit_daily, format: compact },
    { label: '今日费用', used: summary.used_cost_today_usd, limit: summary.cost_limit_daily_usd, format: money },
    { label: '累计费用', used: summary.used_cost_total_usd, limit: summary.cost_limit_total_usd, format: money },
  ]
  return <Panel className="grid gap-5 p-5 sm:p-6">
    <div className="flex flex-wrap items-start justify-between gap-4">
      <div className="min-w-0">
        <div className="flex flex-wrap items-center gap-2"><KeyRound aria-hidden="true" className="size-5 text-primary" /><h1 className="text-[22px] font-semibold tracking-tight">{summary.key_name}</h1><Badge tone={status.tone}><span className="size-1.5 rounded-full bg-current" />{status.label}</Badge></div>
        <p className="mt-1.5 text-sm text-muted-foreground"><code>{summary.key_prefix}…</code> · 由 {summary.account_name} 发放 · 最近调用 {summary.last_used_at ? formatShortDate(summary.last_used_at) : '暂无'}</p>
      </div>
      <Button variant="outline" size="sm" onClick={onRefresh} disabled={refreshing}><RefreshCw aria-hidden="true" className={refreshing ? 'animate-spin' : ''} />刷新</Button>
    </div>
    <p role="status" className={`flex gap-2 rounded-md p-3 text-sm ${{ success: 'bg-success/10 text-success', warning: 'bg-warning/10 text-warning', danger: 'bg-destructive/10 text-destructive' }[status.tone]}`}><Icon aria-hidden="true" className="mt-0.5 size-4 shrink-0" /><span>{status.detail(summary)}</span></p>
    <div className="grid gap-3 sm:grid-cols-3">
      {quotas.map(quota => <Quota key={quota.label} {...quota} />)}
    </div>
    <p className="text-xs text-muted-foreground">每分钟最多 {summary.requests_per_minute ? `${whole(summary.requests_per_minute)} 次请求` : '不限请求数'} · 最多 {whole(summary.concurrency_limit)} 个请求同时进行 · 今日额度在 {formatShortDate(summary.day_ends_at)} 重置</p>
  </Panel>
}

/** One limit as a tile: what is used, then the cap and its progress, or a note that there is none. */
function Quota({ label, used, limit, format }: { label: string; used: number; limit: number | null; format: (value: number) => string }) {
  const ratio = limit ? Math.min(1, used / limit) : 0
  return <div className="grid content-start gap-1.5 rounded-lg border border-border bg-background/40 p-3">
    <span className="text-xs text-muted-foreground">{label}</span>
    <span className="text-lg font-semibold tabular-nums">{format(used)}</span>
    {limit != null ? <>
      <div className="h-1.5 overflow-hidden rounded-full bg-muted" role="progressbar" aria-label={`${label}已用比例`} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(ratio * 100)}>
        <span className={`block h-full rounded-full ${ratio >= 1 ? 'bg-destructive' : ratio >= 0.8 ? 'bg-warning' : 'bg-primary'}`} style={{ width: `${Math.max(ratio * 100, used > 0 ? 2 : 0)}%` }} />
      </div>
      <span className="text-xs text-muted-foreground tabular-nums">上限 {format(limit)} · 已用 {Math.round(ratio * 100)}%</span>
    </> : <span className="text-xs text-muted-foreground">不设上限</span>}
  </div>
}

/** How the issuer raised, changed and reset the total limit, with their notes. Hidden until there is something to show. */
function QuotaPanel({ secret, id, capped }: { secret: string; id: string; capped: boolean }) {
  const events = useQuery({ queryKey: ['key-holder', 'quota', id], queryFn: ({ signal }) => holderApi<QuotaEvent[]>('/quota-events', secret, signal) })
  if (!capped && !events.data?.length) return null
  return <Panel className="grid content-start gap-4 p-5">
    <div><h2 className="font-medium">额度记录</h2><p className="mt-1 text-xs text-muted-foreground">发放方增加、调整总费用额度或重置用量的记录，以及他们留下的备注。</p></div>
    <QuotaHistory events={events.data} pending={events.isPending} error={events.error} retry={() => events.refetch()} initial={5} />
  </Panel>
}

function ModelsPanel({ models }: { models: KeySummary['models'] }) {
  const [copied, setCopied] = useState<string | null>(null)
  const groups = (Object.keys(protocolLabels) as Protocol[]).filter(protocol => models[protocol]?.length)
  async function copy(name: string) { try { await copyText(name); setCopied(name) } catch { setCopied(null) } }
  return <Panel className="grid content-start gap-4 p-5">
    <div><h2 className="font-medium">可用模型</h2><p className="mt-1 text-xs text-muted-foreground">点击模型名即可复制。客户端只能请求所用协议下列出的模型。</p></div>
    {groups.length ? groups.map(protocol => <section key={protocol} className="grid gap-2" aria-label={`${protocolLabels[protocol]} 协议模型`}>
      <h3 className="text-xs font-medium text-muted-foreground">{protocolLabels[protocol]} · {models[protocol]!.length} 个</h3>
      <div className="flex flex-wrap gap-1.5">{models[protocol]!.map(name => <button key={name} type="button" onClick={() => copy(name)} title="复制模型名" className="inline-flex items-center gap-1 rounded-md border border-border bg-muted/30 px-2 py-1 font-mono text-xs hover:border-primary/50 hover:text-primary">{copied === name ? <Check aria-hidden="true" className="size-3" /> : null}{name}</button>)}</div>
    </section>) : <p className="text-sm text-muted-foreground">这把 Key 当前没有可调用的模型。如果状态正常，请联系发放方开放模型。</p>}
  </Panel>
}

function ConnectPanel({ secret, models }: { secret: string; models: KeySummary['models'] }) {
  const [clientId, setClientId] = useState<ClientId>('claude-code')
  const [copy, setCopy] = useState<'idle' | 'copied' | 'error'>('idle')
  const origin = useOrigin()
  const names = Object.values(models).flat()
  const model = exampleModel(names, modelPrefix[clientId])
  const snippets = clientSnippets({ origin, key: keyToken, model }, { curl: true })
  const snippet = snippets.find(item => item.id === clientId) ?? snippets[0]
  const missing = snippet.expects && names.length > 0 && !names.some(name => name.startsWith(snippet.expects!.prefix))
  async function copySnippet() {
    try { await copyText(snippet.code.split(keyToken).join(secret)); setCopy('copied') } catch { setCopy('error') }
  }
  return <Panel className="grid content-start gap-4 p-5">
    <div><h2 className="font-medium">接入方式</h2><p className="mt-1 text-xs text-muted-foreground">接口地址 <code>{origin}</code>（OpenAI 兼容客户端加 <code>/v1</code>）</p></div>
    <div className="overflow-hidden rounded-lg border border-border">
      <div className="flex gap-1 overflow-x-auto border-b border-border bg-muted/30 p-1" role="tablist" aria-label="选择客户端">
        {snippets.map(item => <button key={item.id} type="button" role="tab" aria-selected={item.id === clientId} onClick={() => { setClientId(item.id); setCopy('idle') }} className="whitespace-nowrap rounded-md px-3 py-1.5 text-sm text-muted-foreground hover:text-foreground aria-selected:bg-background aria-selected:font-medium aria-selected:text-foreground aria-selected:shadow-sm">{item.name}</button>)}
      </div>
      <div role="tabpanel" className="grid gap-3 p-3">
        <div className="flex items-center justify-between gap-3">
          <span className="inline-flex items-center gap-1.5 text-xs text-muted-foreground"><Code2 aria-hidden="true" className="size-3.5" />{snippet.label}</span>
          <Button type="button" variant="outline" size="sm" onClick={copySnippet}>{copy === 'copied' ? <Check aria-hidden="true" /> : <Copy aria-hidden="true" />}{copy === 'copied' ? '已复制' : '复制配置'}</Button>
        </div>
        <pre tabIndex={0} aria-label={`${snippet.name} 接入配置`} className="overflow-x-auto rounded-md bg-muted/50 p-3 text-xs leading-5"><code>{snippet.code.split(keyToken).map((part, index) => <Fragment key={index}>{index > 0 && <span className="connect-key-token" title="点击「复制配置」时自动填入完整 Key">复制时自动填入 Key</span>}{part}</Fragment>)}</code></pre>
        <p className="text-xs leading-5 text-muted-foreground" role="status">{copy === 'copied' ? '已复制，配置中包含完整 Key，请勿分享。' : copy === 'error' ? '复制失败，请重试。' : snippet.note.replace('需在后台开放同名模型', '需要这把 Key 有同名模型').replace('模型名需在后台已开放', '模型名需在上方可用模型中')}</p>
        {missing && <p className="flex gap-2 rounded-md bg-warning/10 p-3 text-sm text-warning"><CircleAlert aria-hidden="true" className="mt-0.5 size-4 shrink-0" /><span>{snippet.name} 默认请求 {snippet.expects!.prefix}* 模型，这把 Key 当前没有这类模型。请在客户端里改用上方列出的模型，或联系发放方开放。</span></p>}
      </div>
    </div>
    {CLOUD && <section aria-labelledby="holder-desktop-title" className="desktop-promo flex flex-wrap items-center gap-3 rounded-lg border border-primary/30 p-3 sm:flex-nowrap">
      <BrandMark />
      <div className="min-w-0 flex-1">
        <h3 id="holder-desktop-title" className="text-sm font-medium">用 VAIR Desktop 自动配置</h3>
        <p className="mt-0.5 text-xs leading-5 text-muted-foreground">无需帐号，粘贴这把 Key 即可一键写入 Claude Code、Codex、Gemini CLI 的配置，并检测连接。</p>
      </div>
      <Button type="button" variant="outline" size="sm" className="shrink-0" onClick={() => window.open('/download', '_blank', 'noopener')}><Download aria-hidden="true" />下载客户端</Button>
    </section>}
  </Panel>
}

function UsagePanel({ secret, id }: { secret: string; id: string }) {
  const [selectedRequest, setSelectedRequest] = useState<string | null>(null)
  const requestTrigger = useRef<HTMLButtonElement | null>(null)
  const [range, setRange] = useState<UsageRange>('7d')
  const [status, setStatus] = useState<'all' | 'success' | 'error'>('all')
  const [page, setPage] = useState(1)
  const offset = -new Date().getTimezoneOffset()
  const usage = useQuery({
    queryKey: ['key-holder', 'usage', id, range],
    queryFn: ({ signal }) => holderApi<KeyUsage>(`/usage?range=${range}&utc_offset_minutes=${offset}`, secret, signal),
    placeholderData: keepPreviousData,
    retry: false,
  })
  const events = useQuery({
    queryKey: ['key-holder', 'events', id, range, status, page],
    queryFn: ({ signal }) => holderApi<KeyEvents>(`/events?range=${range}&utc_offset_minutes=${offset}&page=${page}&limit=${PAGE_SIZE}${status === 'all' ? '' : `&status=${status}`}`, secret, signal),
    placeholderData: keepPreviousData,
    retry: false,
  })
  const totals = usage.data?.totals
  const total = events.data?.total ?? 0
  const pageCount = Math.max(1, Math.ceil(total / PAGE_SIZE))
  const chart = usage.data?.trend.map(point => ({ ...point, label: bucketLabel(point.start, usage.data!.bucket_seconds) })) ?? []
  return <section className="grid gap-5" aria-label="用量">
    <div className="flex flex-wrap items-center justify-between gap-3">
      <h1 className="text-sm text-muted-foreground">按所选时间范围统计这把 Key 的调用</h1>
      <div className="w-40"><Choice aria-label="数据范围" value={range} onChange={value => { setRange(value as UsageRange); setPage(1) }} options={rangeOptions} /></div>
    </div>
    <ErrorBox error={usage.error ?? events.error} retry={() => { usage.refetch(); events.refetch() }} />
    {totals ? <StatGrid>
      <StatCard label="请求数" value={whole(totals.requests)} detail={totals.requests ? `${(totals.successful / totals.requests * 100).toFixed(1)}% 成功` : '暂无请求'} icon={<Activity aria-hidden="true" />} />
      <StatCard label="Token" value={compact(totals.input_tokens + totals.output_tokens)} detail={`输入 ${compact(totals.input_tokens)} · 输出 ${compact(totals.output_tokens)}`} icon={<Zap aria-hidden="true" />} tone="success" />
      <StatCard label="费用" value={usageCost(totals.known_cost_usd)} detail="按调用时价格计算" icon={<Coins aria-hidden="true" />} tone="warning" />
      <StatCard label="平均耗时" value={totals.avg_duration_ms == null ? '—' : duration(totals.avg_duration_ms)} icon={<Clock3 aria-hidden="true" />} />
    </StatGrid> : !usage.isError && <div className="h-28 animate-pulse rounded-xl bg-muted" />}
    {usage.data && <div className="grid gap-5 lg:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
      <Panel className="p-5">
        <h3 className="text-sm font-medium">请求趋势</h3>
        {usage.data.totals.requests ? <div className="mt-3 h-56">
          <ResponsiveContainer width="100%" height="100%">
            <BarChart data={chart} margin={{ top: 8, right: 8, left: 0, bottom: 0 }}>
              <CartesianGrid stroke="rgba(226,246,244,.07)" vertical={false} />
              <XAxis dataKey="label" tick={{ fill: '#9aaeb3', fontSize: 11 }} tickLine={false} axisLine={false} interval="preserveStartEnd" minTickGap={12} height={20} />
              <YAxis width={40} allowDecimals={false} tick={{ fill: '#9aaeb3', fontSize: 11 }} tickLine={false} axisLine={false} tickFormatter={value => usageAxisTick(Number(value))} />
              <Tooltip cursor={{ fill: 'rgba(120,217,208,.06)' }} contentStyle={{ background: '#17232a', border: '1px solid rgba(226,246,244,.14)', borderRadius: 8, color: '#eef6f5' }} labelFormatter={(_, payload) => payload?.[0] ? formatDate(payload[0].payload.start) : ''} formatter={(value, _, item) => [`${whole(Number(value))} 次 · ${compact(item.payload.tokens)} Token · ${usageCost(item.payload.estimated_cost_usd)}`, '请求']} />
              <Bar dataKey="requests" fill="#78d9d0" radius={[4, 4, 0, 0]} maxBarSize={28} />
            </BarChart>
          </ResponsiveContainer>
        </div> : <Empty title="暂无请求" description="在客户端用这把 Key 发起请求后，这里会显示趋势。" />}
      </Panel>
      <Panel className="p-5">
        <h3 className="text-sm font-medium">按模型</h3>
        {usage.data.models.length ? <table className="mt-3 w-full text-sm"><thead className="text-xs text-muted-foreground"><tr><th className="pb-2 text-left font-medium">模型</th><th className="pb-2 text-right font-medium">请求</th><th className="pb-2 text-right font-medium">Token</th><th className="pb-2 text-right font-medium">费用</th></tr></thead><tbody>{usage.data.models.map(row => <tr key={row.model} className="border-t border-border"><td className="max-w-0 truncate py-2 pr-2" title={row.model}>{row.model}</td><td className="py-2 text-right tabular-nums">{whole(row.requests)}</td><td className="py-2 text-right tabular-nums">{compact(row.tokens)}</td><td className="py-2 text-right tabular-nums">{usageCost(row.estimated_cost_usd ?? 0)}</td></tr>)}</tbody></table> : <p className="mt-3 text-sm text-muted-foreground">暂无数据</p>}
      </Panel>
    </div>}
    <Panel className="min-w-0 overflow-hidden">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-border p-4">
        <div><h3 className="text-sm font-medium">调用明细</h3><p className="mt-1 text-xs text-muted-foreground">网关拒绝的请求（如超出额度、模型无权限）不会出现在这里，错误原因会直接返回给客户端。</p></div>
        <div className="w-36"><Choice aria-label="请求状态" value={status} onChange={value => { setStatus(value as typeof status); setPage(1) }} options={[{ value: 'all', label: '全部状态' }, { value: 'success', label: '成功请求' }, { value: 'error', label: '失败请求' }]} /></div>
      </div>
      {events.data?.items.length ? <div className="overflow-x-auto"><table className="w-full text-left text-sm"><thead className="bg-muted/30 text-xs text-muted-foreground"><tr>{['时间', '模型', '协议', '状态', '耗时', 'Token', '费用'].map(label => <th key={label} className="whitespace-nowrap px-4 py-3 font-medium">{label}</th>)}</tr></thead><tbody>{events.data.items.map(row => <tr key={row.id} className="cursor-pointer border-t border-border hover:bg-muted/30" onClick={event => { requestTrigger.current = event.currentTarget.querySelector<HTMLButtonElement>('[data-request-details]'); setSelectedRequest(row.id) }}>
        <Cell muted title={formatDate(row.created_at)}>{formatShortDate(row.created_at)}</Cell>
        <Cell><button type="button" data-request-details aria-haspopup="dialog" aria-label={`查看 ${row.model} 请求详情`} className="min-h-10 rounded-sm text-left font-medium hover:text-primary focus-visible:outline-2 focus-visible:outline-ring">{row.model}</button>{row.source === 'monitor' && <span className="ml-1.5" title="通过 VAIR Key 渠道发起的检查或质量监控"><Badge>监控</Badge></span>}</Cell>
        <Cell muted>{protocolLabels[row.protocol] ?? row.protocol}</Cell>
        <Cell><Badge tone={row.status < 300 ? 'success' : row.status < 500 ? 'warning' : 'danger'}><span className="size-1.5 rounded-full bg-current" />{row.status}</Badge></Cell>
        <Cell muted>{duration(row.duration_ms)}{row.stream && row.first_token_ms != null ? <span className="block text-xs">首个输出 {duration(row.first_token_ms)}</span> : row.ttfb_ms != null && <span className="block text-xs">响应头 {duration(row.ttfb_ms)}</span>}</Cell>
        <Cell>{row.input_tokens == null && row.output_tokens == null ? '—' : <span title={`输入 ${whole(row.input_tokens ?? 0)} · 输出 ${whole(row.output_tokens ?? 0)}${row.cached_tokens ? ` · 缓存 ${whole(row.cached_tokens)}` : ''}`}>{compact((row.input_tokens ?? 0) + (row.output_tokens ?? 0))}</span>}</Cell>
        <Cell>{row.cost_usd != null ? `$${row.cost_usd.toFixed(6)}` : <span className="text-muted-foreground">—</span>}</Cell>
      </tr>)}</tbody></table></div> : events.isPending ? <div className="h-40 animate-pulse bg-muted/40" /> : <Empty title="没有调用记录" description="调整时间范围或状态筛选后再试。" />}
      <div className="flex items-center justify-between border-t border-border px-4 py-3 text-xs text-muted-foreground">
        <span>共 {whole(total)} 条</span>
        <div className="flex items-center gap-2"><Button variant="outline" size="sm" onClick={() => setPage(current => Math.max(1, current - 1))} disabled={page <= 1}>上一页</Button><span className="tabular-nums">{page} / {pageCount}</span><Button variant="outline" size="sm" onClick={() => setPage(current => Math.min(pageCount, current + 1))} disabled={page >= pageCount}>下一页</Button></div>
      </div>
    </Panel>
    <KeyRequestDetails requestId={selectedRequest} cacheId={id} load={(eventId, signal) => holderApi<KeyRequestDetail>(`/events/${encodeURIComponent(eventId)}`, secret, signal)} onClose={() => setSelectedRequest(null)} returnFocus={() => requestTrigger.current?.focus({ preventScroll: true })} />
  </section>
}

function Cell({ children, muted, title }: { children: ReactNode; muted?: boolean; title?: string }) {
  return <td title={title} className={`whitespace-nowrap px-4 py-3 tabular-nums${muted ? ' text-muted-foreground' : ''}`}>{children}</td>
}

function bucketLabel(start: string, bucketSeconds: number) {
  const date = new Date(start)
  if (bucketSeconds < 86_400) return `${String(date.getHours()).padStart(2, '0')}:00`
  if (bucketSeconds >= 30 * 86_400) return `${date.getFullYear()}/${date.getMonth() + 1}`
  return `${date.getMonth() + 1}/${date.getDate()}`
}
