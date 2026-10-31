import { Fragment, useState } from 'react'
import { useMutation, useQuery } from '@tanstack/react-query'
import { useSearchParams } from 'react-router-dom'
import { ChevronLeft, ChevronRight, Fingerprint, Plus, Settings2, ShieldAlert, Trash2 } from 'lucide-react'
import { api, ApiError, formatDate, formatShortDate, jsonBody, type QualityMonitor } from '../lib/api'
import { queryClient, useAuthenticityMonitors, useChannels, useQualityMonitors, useUser } from '../lib/queries'
import { Badge, Choice, Confirm, Empty, ErrorBox, Field, FormSheet, HelpTip, Loading, Panel, WarningTip } from '../components/common'
import { Confidence, DeepCheckForm, Report, VerdictBadge, useDeepChecks } from '../components/deep-check'
import { ModelSelect } from '../components/model-select'
import { Button } from '../components/ui/button'
import { Checkbox } from '../components/ui/checkbox'
import { Input } from '../components/ui/input'
import { channelModelOptions, ModelMapping, MonitorTarget } from './quality-monitoring'
import { verdictOf, type DeepCheckJob, type DeepCheckOptions } from '../lib/authenticity'

type HistoryItem = DeepCheckJob & { monitor_id: string; model: string; channel_id: string; channel_name: string; monitor_deleted: boolean }
type Settings = { claimed_model: string | null; schedule_enabled: boolean; interval_seconds: number; samples_per_run: number; window_days: number; daily_sample_limit: number; route_policy: 'observe' | 'deprioritize' | 'exclude' }

const intervals = [{ value: '3600', label: '1 小时' }, { value: '10800', label: '3 小时' }, { value: '21600', label: '6 小时' }, { value: '43200', label: '12 小时' }, { value: '86400', label: '24 小时' }]
const windows = [{ value: '3', label: '3 天' }, { value: '7', label: '7 天' }, { value: '14', label: '14 天' }, { value: '30', label: '30 天' }]
const policies = [{ value: 'observe', label: '仅观察' }, { value: 'deprioritize', label: '疑似或确认不符时降低优先级' }, { value: 'exclude', label: '确认不符时剔除（疑似时降低优先级）' }]
const mismatch = (verdict: string | null | undefined) => verdict === 'suspected' || verdict === 'confirmed'
const fresh = (until: string | null) => !!until && new Date(until).getTime() > Date.now()

function routingLabel(m: QualityMonitor) {
  if (m.route_policy === 'observe') return '仅观察'
  if (fresh(m.window_valid_until) && m.window_verdict === 'confirmed') return m.route_policy === 'exclude' ? '已剔除该模型' : '已降低该模型优先级'
  if (fresh(m.window_valid_until) && m.window_verdict === 'suspected') return '已降低该模型优先级'
  return m.route_policy === 'exclude' ? '确认不符时剔除' : '不符时降低优先级'
}

/** The deep check and the rolling window disagree: the relay may treat concentrated probes differently. */
function disagreement(m: QualityMonitor) {
  if (!m.window_verdict || !m.identity_verdict || !m.identity_checked_at) return null
  if (Date.now() - new Date(m.identity_checked_at).getTime() > 7 * 86400_000) return null
  const window = mismatch(m.window_verdict)
  const deep = mismatch(m.identity_verdict)
  if (window === deep) return null
  return `深度检测为「${verdictOf(m.identity_verdict)?.label}」，定时抽样为「${verdictOf(m.window_verdict)?.label}」。渠道可能在集中检测时与平时表现不同。`
}

/** The verdict a monitor shows: the sampling window while it is current, otherwise the newer of the two sources. */
function currentVerdict(m: QualityMonitor) {
  const window = m.window_verdict ? { verdict: m.window_verdict, source: '定时抽样', detail: m.window_evidence?.window ? `${m.window_evidence.window.samples} 个样本 / ${m.window_evidence.window.days} 天` : undefined, evidence: m.window_evidence, at: m.window_checked_at } : null
  const deep = m.identity_verdict ? { verdict: m.identity_verdict, source: '深度检测', detail: undefined, evidence: m.identity_evidence, at: m.identity_checked_at } : null
  if (window && fresh(m.window_valid_until)) return window
  if (window && deep) return (deep.at ?? '') > (window.at ?? '') ? deep : window
  return window ?? deep
}

const MIN_SAMPLES = 8

/** What the list says about a monitor: a plain answer, one line of the most useful detail, and where it came from. */
function identityState(m: QualityMonitor) {
  const current = currentVerdict(m)
  const tip = current ? { source: current.source, at: current.at, reasons: current.evidence && 'reasons' in current.evidence ? current.evidence.reasons ?? [] : [] } : null
  if (!m.channel_enabled) return { label: '渠道已停用', tone: 'neutral' as const, line: null, tip }
  if (!current) return m.schedule_enabled && m.claimed_model ? { label: '累积中', tone: 'neutral' as const, line: '等待首轮抽样', tip } : { label: '尚未检测', tone: 'neutral' as const, line: null, tip }
  const window = current.source === '定时抽样'
  const samples = current.evidence?.valid_samples ?? (window ? m.window_evidence?.window?.samples : undefined)
  const count = samples != null ? `${samples} 个样本` : null
  const best = current.evidence?.best_model && current.evidence.best_model !== (current.evidence.claimed_model ?? m.claimed_model) ? current.evidence.best_model : null
  switch (current.verdict) {
    case 'match': return { label: '相符', tone: 'success' as const, line: count, tip }
    case 'family_only': return { label: '仅确认厂商', tone: 'neutral' as const, line: `${count ? `${count}，` : ''}无法区分同家族`, tip }
    case 'suspected': return { label: '疑似不符', tone: 'warning' as const, line: best ? `更像 ${best}` : count, tip }
    case 'confirmed': return { label: '确认不符', tone: 'danger' as const, line: best ? `实际为 ${best}` : count, tip }
    case 'insufficient': return window && (samples ?? 0) < MIN_SAMPLES ? { label: '累积中', tone: 'neutral' as const, line: `已有 ${samples ?? 0}/${MIN_SAMPLES} 个样本`, tip } : { label: '无法判断', tone: 'warning' as const, line: '有效样本不足', tip }
    case 'ambiguous': return { label: '无法判断', tone: 'warning' as const, line: '证据互相矛盾', tip }
    case 'unsupported': return { label: '暂不支持', tone: 'neutral' as const, line: '申报模型没有启用的基准', tip }
    default: return { label: verdictOf(current.verdict)?.label ?? current.verdict, tone: 'neutral' as const, line: count, tip }
  }
}

const running = (m: QualityMonitor) => m.job_status === 'queued' || m.job_status === 'running'
const needsAttention = (m: QualityMonitor) => mismatch(currentVerdict(m)?.verdict) || identityState(m).label === '无法判断' || !!disagreement(m) || !!m.schedule_error || m.job_status === 'failed' || (m.schedule_enabled && !m.claimed_model)

function settingsOf(m: QualityMonitor): Settings {
  return { claimed_model: m.claimed_model, schedule_enabled: m.schedule_enabled, interval_seconds: m.interval_seconds, samples_per_run: m.samples_per_run, window_days: m.window_days, daily_sample_limit: m.daily_sample_limit, route_policy: m.route_policy }
}

const newSettings: Settings = { claimed_model: null, schedule_enabled: true, interval_seconds: 21600, samples_per_run: 2, window_days: 7, daily_sample_limit: 0, route_policy: 'observe' }

/** Authenticity monitors: one per channel and model, sampled on a schedule and checked on demand. */
export function IdentityMonitoring() {
  const user = useUser()
  const monitors = useAuthenticityMonitors()
  const services = useQualityMonitors()
  const channels = useChannels()
  const [params, setParams] = useSearchParams()
  const scopedChannel = params.get('channel') ?? undefined
  const search = params.get('identity_search') ?? ''
  const filter = params.get('identity_filter') ?? 'all'
  // Older links used ?deep=<combined monitor>; since the split that id is the
  // service monitor, so open the authenticity monitor of the same channel and model.
  const legacy = params.get('deep')
  const legacyService = legacy ? services.data?.find(m => m.id === legacy) : undefined
  const detailId = params.get('identity') ?? (legacyService ? monitors.data?.find(m => m.channel_id === legacyService.channel_id && m.model === legacyService.model)?.id : legacy) ?? undefined
  // Leaving an older ?deep= link keeps the page on this tab.
  const setParam = (name: string, value: string) => setParams(current => { const next = new URLSearchParams(current); if (next.has('deep')) { next.delete('deep'); next.set('tab', 'identity') } if (value) next.set(name, value); else next.delete(name); return next }, { replace: true })
  const [editor, setEditor] = useState<{ monitor: QualityMonitor | null } | null>(null)
  const [remove, setRemove] = useState<QualityMonitor | null>(null)
  const [notice, setNotice] = useState('')
  const [deepFor, setDeepFor] = useState<QualityMonitor | null>(null)
  const openRecord = (monitorId: string) => setParams(current => { const next = new URLSearchParams(current); next.delete('deep'); next.set('tab', 'identity'); next.set('identity', monitorId); return next }, { replace: true })
  const refresh = () => Promise.all([queryClient.invalidateQueries({ queryKey: ['quality-monitors', user.account_id] }), queryClient.invalidateQueries({ queryKey: ['authenticity-history', user.account_id] })])
  const deletion = useMutation({ mutationFn: (id: string) => api(`/quality-monitors/${id}`, { method: 'DELETE' }), onSuccess: async () => { setRemove(null); await refresh() } })
  const visible = (monitors.data ?? []).filter(m => (!scopedChannel || m.channel_id === scopedChannel) && `${m.channel_name} ${m.model} ${m.upstream_model}`.toLowerCase().includes(search.toLowerCase())
    && (filter === 'all' || filter === 'scheduled' && m.schedule_enabled || filter === 'manual' && !m.schedule_enabled || filter === 'attention' && needsAttention(m)))
  const detail = monitors.data?.find(m => m.id === detailId)
  const openAdd = () => { setNotice(''); setEditor({ monitor: null }) }
  return <>
    <div className="mt-5 flex flex-wrap items-center justify-between gap-3"><div><h2 className="font-semibold">真伪检测</h2><p className="mt-1 text-xs text-muted-foreground">检查渠道返回的是不是它声称的模型：用随机数字指纹和 Tokenizer 计数与官方基准比对。可以定时少量抽样、在窗口内累积判定，也可以随时发起深度检测。</p></div><Button onClick={openAdd}><Plus aria-hidden="true" className="size-4" />添加真伪检测</Button></div>
    <div className="mt-3"><ErrorBox error={monitors.error ?? channels.error} retry={() => void refresh()} /></div>
    {notice && <p role="status" className="mt-3 text-sm text-primary">{notice}</p>}
    {monitors.isPending ? <Loading /> : !monitors.data ? null : !monitors.data.length ? <Panel className="mt-4"><Empty title="还没有真伪检测" description="选择渠道和一个具体模型，设置申报模型后可以开启定时抽样，或随时发起深度检测。" action={<Button onClick={openAdd}>添加真伪检测</Button>} /></Panel> : <Panel className="mt-4 overflow-hidden">
      <div className="flex flex-wrap items-center gap-2 border-b border-border p-4"><Input name="identity_search" aria-label="搜索渠道或模型" autoComplete="off" placeholder="搜索渠道或模型…" className="w-full sm:max-w-64" value={search} onChange={e => setParam('identity_search', e.target.value)} /><div className="flex flex-wrap gap-1">{[['all', '全部'], ['attention', '需关注'], ['scheduled', '定时开启'], ['manual', '仅手动']].map(([value, label]) => <Button key={value} size="sm" variant={filter === value ? 'secondary' : 'ghost'} aria-pressed={filter === value} onClick={() => setParam('identity_filter', value)}>{label}</Button>)}</div><span className="ml-auto text-xs text-muted-foreground">{visible.length} 条监控</span></div>
      <div className="overflow-x-auto"><table className="w-full min-w-[1000px] text-left text-sm"><thead className="bg-muted/30 text-xs text-muted-foreground"><tr>{['渠道 / 模型', '最近状态', '申报模型', '定时抽样', '最近 / 下次检测', '路由影响', '操作'].map(title => <th key={title} className={`px-4 py-3 font-medium${title === '操作' ? ' text-right' : ''}`}>{title}</th>)}</tr></thead><tbody>
        {visible.map(m => {
          const state = identityState(m)
          const checked = [m.window_checked_at, m.identity_checked_at].filter((v): v is string => !!v).sort().at(-1)
          const conflict = disagreement(m)
          return <tr key={m.id} className="border-t border-border hover:bg-muted/30">
            <td className="max-w-60 px-4 py-4"><div className="font-medium">{m.channel_name}</div><div className="mt-1 break-all text-xs">{m.model}</div>{m.model !== m.upstream_model && <div className="mt-1 break-all text-xs text-muted-foreground">→ {m.upstream_model}</div>}</td>
            <td className="px-4 py-4"><div className="flex items-center gap-1"><button type="button" className="rounded-md focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" title="查看详情" onClick={() => openRecord(m.id)}><Badge tone={state.tone}>{state.label}</Badge></button>
                {conflict ? <WarningTip label="两种检测结论不一致"><p>{conflict}</p>{state.tip && <div className="mt-1 text-muted-foreground"><p className="font-medium">依据{state.tip.source}{state.tip.at ? ` · ${formatShortDate(state.tip.at)}` : ''}</p>{!!state.tip.reasons.length && <ul className="mt-1 grid list-disc gap-0.5 pl-4">{state.tip.reasons.map(r => <li key={r}>{r}</li>)}</ul>}</div>}</WarningTip> : state.tip && <HelpTip label="结论依据"><p className="font-medium">依据{state.tip.source}{state.tip.at ? ` · ${formatShortDate(state.tip.at)}` : ''}</p>{!!state.tip.reasons.length && <ul className="mt-1 grid list-disc gap-0.5 pl-4">{state.tip.reasons.map(r => <li key={r}>{r}</li>)}</ul>}</HelpTip>}</div>
              {state.line && <div className="mt-1 break-all text-xs text-muted-foreground">{state.line}</div>}
              {running(m) && <div role="status" className="mt-1 text-xs text-primary">{m.job_status === 'queued' ? '等待检测…' : '检测中…'}</div>}
              {m.schedule_error && <div className="mt-1 max-w-48 text-xs text-warning">{m.schedule_error}</div>}</td>
            <td className="max-w-48 break-all px-4 py-4 text-xs">{m.claimed_model ?? <span className="text-warning">未设置</span>}</td>
            <td className="px-4 py-4"><Badge tone={m.schedule_enabled ? 'success' : 'neutral'}>{m.schedule_enabled ? '已开启' : '仅手动'}</Badge><div className="mt-1 text-xs text-muted-foreground">{!m.channel_enabled ? '渠道停用，已暂停' : m.schedule_enabled ? `每 ${intervals.find(i => Number(i.value) === m.interval_seconds)?.label ?? `${Math.round(m.interval_seconds / 3600)} 小时`} · 每轮 ${m.samples_per_run} 个` : '不自动执行'}</div></td>
            <td className="whitespace-nowrap px-4 py-4 text-xs tabular-nums text-muted-foreground"><div title={checked ? formatDate(checked) : undefined}>{checked ? formatShortDate(checked) : '尚未检测'}</div><div className="mt-1">下次：{m.channel_enabled && m.schedule_enabled && m.next_run_at ? formatShortDate(m.next_run_at) : '—'}</div></td>
            <td className="px-4 py-4 text-xs">{routingLabel(m)}</td>
            <td className="px-4 py-4"><div className="flex justify-end gap-1"><Button size="sm" variant="ghost" disabled={!m.channel_enabled || running(m)} title={running(m) ? '已有检测在进行，完成后可发起深度检测' : undefined} onClick={() => setDeepFor(m)}><Fingerprint aria-hidden="true" className="size-4" />深度检测</Button><Button size="sm" variant="ghost" onClick={() => openRecord(m.id)}>详情</Button><Button size="sm" variant="ghost" onClick={() => setEditor({ monitor: m })}><Settings2 aria-hidden="true" className="size-4" />编辑</Button><Button size="icon" variant="ghost" aria-label={`删除 ${m.channel_name} / ${m.model} 真伪检测`} onClick={() => { deletion.reset(); setRemove(m) }}><Trash2 aria-hidden="true" className="size-4" /></Button></div></td>
          </tr>
        })}
        {!visible.length && <tr><td colSpan={7} className="p-8 text-center text-muted-foreground">没有匹配的监控，试试调整搜索或筛选条件。</td></tr>}
      </tbody></table></div>
    </Panel>}
    <IdentityHistory key={scopedChannel ?? 'all'} channelId={scopedChannel} />
    <FormSheet open={!!detailId} onOpenChange={open => { if (!open) { setParams(current => { const next = new URLSearchParams(current); next.delete('identity'); next.delete('deep'); next.set('tab', 'identity'); return next }, { replace: true }); void refresh() } }} title="真伪检测详情" description={detail ? `${detail.channel_name} / ${detail.model}` : '检测渠道是否提供申报的模型'}>
      {detail && <MonitorDetail key={detail.id} monitor={detail} onDeepCheck={() => setDeepFor(detail)} />}
    </FormSheet>
    <FormSheet open={!!deepFor} onOpenChange={open => { if (!open) setDeepFor(null) }} title="发起深度检测" description={deepFor ? `${deepFor.channel_name} / ${deepFor.model}` : ''}>
      {deepFor && <DeepCheckForm key={deepFor.id} monitorId={deepFor.id} defaultClaim={deepFor.claimed_model} onCancel={() => setDeepFor(null)} onStarted={() => { const id = deepFor.id; setDeepFor(null); openRecord(id); void refresh() }} />}
    </FormSheet>
    {editor && <SettingsSheet monitor={editor.monitor} onClose={() => setEditor(null)} onSaved={async created => { setEditor(null); if (created) setNotice(created); await refresh() }} />}
    <Confirm open={!!remove} onOpenChange={open => { if (!open && !deletion.isPending) setRemove(null) }} title="删除真伪检测" description={`删除 ${remove?.channel_name ?? ''} / ${remove?.model ?? ''} 的真伪检测？定时抽样和它的路由影响将停止，检测记录保留，服务体检不受影响。`} pending={deletion.isPending} error={deletion.error} onConfirm={() => { if (remove) deletion.mutate(remove.id) }} />
  </>
}

/** Current results of one monitor: the sampling window and the latest deep check. */
function MonitorDetail({ monitor, onDeepCheck }: { monitor: QualityMonitor; onDeepCheck: () => void }) {
  const jobs = useDeepChecks(monitor.id)
  const deep = jobs.data?.find(job => job.source !== 'scheduled')
  const busy = jobs.data?.find(job => job.status === 'queued' || job.status === 'running')
  const sampled = jobs.data?.find(job => job.source === 'scheduled' && job.status === 'completed')
  const conflict = disagreement(monitor)
  return <div className="grid gap-5">
    <section className="grid gap-2"><h3 className="text-sm font-medium">定时抽样</h3>
      {monitor.window_verdict || monitor.schedule_enabled ? <div className="grid gap-2 rounded-lg border border-border p-4 text-sm">
        <div className="flex flex-wrap items-center gap-2">{monitor.window_verdict === 'insufficient' && (monitor.window_evidence?.window?.samples ?? 0) < MIN_SAMPLES ? <Badge>累积中</Badge> : <VerdictBadge verdict={monitor.window_verdict} />}{monitor.window_evidence?.window ? <span className="text-xs text-muted-foreground">{monitor.window_evidence.window.samples} 个样本 / {monitor.window_evidence.window.days} 天{monitor.window_checked_at ? ` · ${formatShortDate(monitor.window_checked_at)}` : ''}</span> : !monitor.window_verdict && <span className="text-xs text-muted-foreground">等待首轮抽样</span>}</div>
        {!!monitor.window_evidence?.reasons?.length && <ul className="grid list-disc gap-0.5 pl-5 text-xs">{monitor.window_evidence.reasons.map(r => <li key={r}>{r}</li>)}</ul>}
        {sampled?.evidence && sampled.verdict === monitor.window_verdict && <Confidence evidence={sampled.evidence} />}
        <p className="text-xs text-muted-foreground">申报 {monitor.claimed_model ?? '未设置'} · 路由影响：{routingLabel(monitor)}</p>
      </div> : <p className="rounded-lg border border-border p-4 text-sm text-muted-foreground">未开启定时抽样。可以在「编辑」中开启。</p>}
    </section>
    <section className="grid gap-2"><div className="flex items-center justify-between gap-2"><h3 className="text-sm font-medium">深度检测</h3><Button size="sm" variant="outline" disabled={!monitor.channel_enabled || !!busy} onClick={onDeepCheck}><Fingerprint aria-hidden="true" className="size-4" />发起深度检测</Button></div>
      {busy?.source === 'scheduled' && <p className="text-xs text-muted-foreground">本轮定时抽样进行中，完成后即可发起深度检测。</p>}
      {jobs.isPending ? <Loading /> : deep ? <Report job={deep} /> : <p className="rounded-lg border border-border p-4 text-sm text-muted-foreground">尚未进行深度检测。</p>}
    </section>
    {conflict && <p className="flex items-start gap-1 text-xs text-warning"><ShieldAlert aria-hidden="true" className="mt-0.5 size-3.5 shrink-0" />{conflict}</p>}
    <p className="text-xs text-muted-foreground">全部检测记录见页面下方「真伪检测记录」，展开一行可查看当次报告。</p>
  </div>
}

/** Adds an authenticity monitor or edits one; both use the same settings, like the service tab. */
function SettingsSheet({ monitor, onClose, onSaved }: { monitor: QualityMonitor | null; onClose: () => void; onSaved: (notice?: string) => Promise<void> }) {
  const user = useUser()
  const channels = useChannels()
  const [target, setTarget] = useState({ channel_id: monitor?.channel_id ?? '', model: monitor?.model ?? '' })
  const model = target.model.trim()
  const channel = channels.data?.find(c => c.id === target.channel_id)
  const options = useQuery({
    queryKey: monitor ? ['deep-check-options', user.account_id, monitor.id] : ['authenticity-options', user.account_id, target.channel_id, model],
    queryFn: ({ signal }) => api<DeepCheckOptions>(monitor ? `/quality-monitors/${monitor.id}/deep-check` : `/authenticity-options?channel_id=${encodeURIComponent(target.channel_id)}&model=${encodeURIComponent(model)}`, { signal }),
    enabled: !!monitor || (!!target.channel_id && !!model),
    staleTime: 30_000,
  })
  const [settings, setSettings] = useState<Settings>(() => monitor ? settingsOf(monitor) : newSettings)
  // A new monitor follows the suggested claim until the user picks one.
  const [claimTouched, setClaimTouched] = useState(!!monitor)
  const claimed = claimTouched ? settings.claimed_model : options.data?.claimed_default ?? null
  const scheduled = settings.schedule_enabled && !!claimed
  const [acknowledged, setAcknowledged] = useState(false)
  const set = <K extends keyof Settings>(key: K, value: Settings[K]) => setSettings(current => ({ ...current, [key]: value }))
  const payload = { ...settings, claimed_model: claimed, schedule_enabled: scheduled, acknowledge_risk: acknowledged }
  const save = useMutation({
    mutationFn: () => monitor ? api(`/quality-monitors/${monitor.id}`, jsonBody(payload, 'PATCH')) : api('/quality-monitors', jsonBody({ channel_id: target.channel_id, model, kind: 'authenticity', authenticity: payload })),
    onSuccess: () => onSaved(monitor ? undefined : scheduled ? '已添加真伪检测，首轮定时抽样即将开始。' : '已添加真伪检测。可以随时发起深度检测。'),
  })
  const selected = options.data?.models.find(m => m.model === claimed)
  const runsPerMonth = 30 * 86400 / settings.interval_seconds
  const monthly = selected?.per_sample_cost_usd != null ? selected.per_sample_cost_usd * settings.samples_per_run * runsPerMonth : null
  const needsAck = scheduled && options.data && !options.data.acknowledged
  const resetsWindow = !!monitor && (claimed !== monitor.claimed_model || settings.window_days !== monitor.window_days)
  const ready = !!monitor || (!!target.channel_id && !!model)
  return <FormSheet open onOpenChange={open => { if (!open && !save.isPending) onClose() }} title={monitor ? '编辑真伪检测' : '添加真伪检测'} description={monitor ? '修改申报模型、定时抽样和路由影响。' : '一条真伪检测对应一个渠道和模型，与服务体检相互独立。'}>
    <form className="grid gap-5" onSubmit={e => { e.preventDefault(); if (ready) save.mutate() }}>
      {monitor ? <MonitorTarget channel={monitor.channel_name} channelEnabled={monitor.channel_enabled} model={monitor.model} upstream={monitor.upstream_model} /> : <>
        <Field label="渠道">{id => <Choice id={id} name="channel_id" value={target.channel_id} options={(channels.data ?? []).map(c => ({ value: c.id, label: `${c.name}${c.enabled ? '' : '（已停用）'}` }))} onChange={value => { setTarget({ channel_id: value, model: '' }); setClaimTouched(false) }} />}</Field>
        <Field label="模型" help="选择或输入该渠道支持的具体模型；每条监控只检测一个模型。">{id => <ModelSelect key={target.channel_id} id={id} name="model" required disabled={!channel} value={target.model} onChange={value => { setTarget({ ...target, model: value }); setClaimTouched(false) }} models={channel ? channelModelOptions(channel) : []} />}</Field>
        {channel && model && <ModelMapping channel={channel} model={model} />}
        {channel && !channel.enabled && <p className="text-xs text-warning">渠道已停用，监控会保留配置，启用渠道后才能执行检测。</p>}
        {channel?.kind === 'vair_key' && <p className="rounded-md bg-warning/10 px-3 py-2 text-xs leading-5 text-warning">这是 VAIR Key 渠道。每个检测样本都通过来源 Key 发出，计入它的请求频率、并发和费用上限，费用由 Key 的所有者承担。</p>}
      </>}
      <Field label="申报模型" hint="渠道声称提供的官方模型。定时抽样和深度检测都与它的基准比较。">{id => !ready ? <p id={id} className="text-sm text-muted-foreground">选择渠道和模型后自动匹配。</p> : options.isPending ? <Loading /> : options.data?.models.length ? <Choice id={id} name="claimed_model" value={claimed ?? ''} onChange={value => { setClaimTouched(true); set('claimed_model', value || null) }} options={options.data.models.map(m => ({ value: m.model, label: m.model }))} /> : <p id={id} className="text-sm text-muted-foreground">系统还没有启用的模型基准。</p>}</Field>
      <div className="grid gap-4 rounded-lg border border-border p-4">
        <label className="flex min-h-11 items-center gap-3 text-sm font-medium"><Checkbox checked={scheduled} disabled={!claimed} onCheckedChange={checked => set('schedule_enabled', checked === true)} />开启定时抽样</label>
        <p className="text-xs leading-5 text-muted-foreground">{claimed ? '每轮只发少量探针，在判定窗口内累积证据，比每次跑满一次深度检测更省钱，也更难被渠道识别。关闭后仍可随时发起深度检测。' : '选择申报模型后才能开启定时抽样。'}</p>
        {scheduled && <>
          <Field label="抽样周期">{id => <Choice id={id} name="interval_seconds" value={String(settings.interval_seconds)} options={intervals} onChange={value => set('interval_seconds', Number(value))} />}</Field>
          <Field label="每轮样本数" hint="1–8。">{id => <Input id={id} name="samples_per_run" type="number" min={1} max={8} required value={settings.samples_per_run} onChange={e => set('samples_per_run', Number(e.target.value))} />}</Field>
          <Field label="判定窗口" hint="用窗口内的抽样样本判定（最多最近 128 个）。修改申报模型或窗口会清空现有结果。">{id => <Choice id={id} name="window_days" value={String(settings.window_days)} options={windows} onChange={value => set('window_days', Number(value))} />}</Field>
          <p className="text-xs text-muted-foreground">{monthly != null ? `按官方价格估算，每月约 $${monthly.toFixed(2)}（${Math.round(runsPerMonth)} 轮 × ${settings.samples_per_run} 个样本）。实际按渠道计费。` : '该模型暂无价格，无法估算每月花费。'}</p>
        </>}
      </div>
      <Field label="路由影响" hint="降低优先级：排在正常渠道之后，正常渠道都不可用时仍会使用。只根据定时抽样的结论执行，两个抽样周期内没有新结果时自动解除。深度检测结果不影响路由。">{id => <Choice id={id} name="route_policy" value={settings.route_policy} options={policies} onChange={value => set('route_policy', value as Settings['route_policy'])} />}</Field>
      <details className="rounded-lg border border-border p-4"><summary className="cursor-pointer text-sm font-medium focus-visible:outline-2 focus-visible:outline-ring">高级设置</summary><div className="mt-4 grid gap-4">
        <Field label="每日样本额度" hint="0 表示不额外限制，按 UTC 自然日统计定时抽样和深度检测的样本。">{id => <Input id={id} name="daily_sample_limit" type="number" min={0} max={100000} required value={settings.daily_sample_limit} onChange={e => set('daily_sample_limit', Number(e.target.value))} />}</Field>
      </div></details>
      {resetsWindow && monitor?.window_verdict && <p className="text-xs text-warning">保存后将清空现有的定时抽样结论，并重新开始累积。</p>}
      {needsAck && <div className="grid gap-3 rounded-lg border border-warning/40 bg-warning/5 p-4 text-sm">
        <p className="flex items-center gap-2 font-medium text-warning"><ShieldAlert aria-hidden="true" className="size-4" />开启定时抽样前请确认</p>
        <ul className="grid list-disc gap-1 pl-5 text-xs leading-5 text-muted-foreground"><li>检测请求可能被中转站识别；部分中转站可能因此对该 Key 区别对待，甚至限制或封禁该 Key。</li><li>检测费用按该渠道的计费由你承担。</li><li>VAIR Key 渠道的检测请求计入来源 Key 的额度。</li></ul>
        <label className="flex min-h-11 items-center gap-3 text-sm"><Checkbox checked={acknowledged} onCheckedChange={checked => setAcknowledged(checked === true)} />我已了解以上风险</label>
      </div>}
      <ErrorBox error={options.error ?? save.error} />
      <div className="flex justify-end gap-2"><Button type="button" variant="outline" disabled={save.isPending} onClick={onClose}>取消</Button><Button type="submit" disabled={save.isPending || !ready || options.isPending || (needsAck && !acknowledged)}>{save.isPending ? '保存中…' : monitor ? '保存设置' : '添加真伪检测'}</Button></div>
    </form>
  </FormSheet>
}

const statusLabels: Record<string, string> = { queued: '排队中', running: '检测中', completed: '已完成', failed: '未完成', cancelled: '已取消' }

function IdentityHistory({ channelId }: { channelId?: string }) {
  const user = useUser()
  const [page, setPage] = useState(1)
  const [expanded, setExpanded] = useState<string | null>(null)
  const query = useQuery({
    queryKey: ['authenticity-history', user.account_id, channelId, page],
    queryFn: ({ signal }) => api<{ items: HistoryItem[]; total: number; page: number; page_size: number }>(`/authenticity-checks?page=${page}&limit=10${channelId ? `&channel_id=${encodeURIComponent(channelId)}` : ''}`, { signal }),
    refetchInterval: query => query.state.data?.items.some(item => item.status === 'queued' || item.status === 'running') ? 3000 : 15000,
  })
  return <Panel className="mt-5 overflow-hidden">
    <div className="border-b border-border p-4"><h2 className="text-sm font-semibold">真伪检测记录</h2><p className="mt-1 text-xs text-muted-foreground">包含已删除监控的历史。定时抽样的结论按当时的判定窗口计算。展开一行查看当次报告。</p></div>
    <ErrorBox error={query.error} retry={() => void query.refetch()} />
    {query.isPending ? <Loading /> : query.data && <>
      {!query.data.items.length ? <p className="p-6 text-center text-sm text-muted-foreground">尚无真伪检测记录，发起检测后将在这里显示。</p> : <div className="overflow-x-auto"><table className="w-full min-w-[860px] text-left text-sm">
        <thead className="bg-muted/30 text-xs text-muted-foreground"><tr>{['时间', '类型', '渠道 / 模型', '申报模型', '结论', '有效样本', '状态', '详情'].map(title => <th key={title} className="px-4 py-3 font-medium">{title}</th>)}</tr></thead>
        <tbody>{query.data.items.map(item => <Fragment key={item.id}><tr className="border-t border-border">
          <td className="whitespace-nowrap px-4 py-3 text-xs tabular-nums text-muted-foreground" title={formatDate(item.created_at)}>{formatShortDate(item.created_at)}</td>
          <td className="px-4 py-3 text-xs">{item.source === 'scheduled' ? '定时抽样' : '深度检测'}</td>
          <td className="max-w-56 px-4 py-3"><div>{item.channel_name}</div><div className="break-all text-xs text-muted-foreground">{item.model}</div>{item.monitor_deleted && <Badge>已删除</Badge>}</td>
          <td className="max-w-48 break-all px-4 py-3 text-xs">{item.claimed_model ?? '—'}</td>
          <td className="px-4 py-3"><VerdictBadge verdict={item.verdict} /></td>
          <td className="px-4 py-3 text-xs tabular-nums">{item.valid_samples}/{item.completed_samples}{item.stopped_early ? ' · 提前结束' : ''}</td>
          <td className="px-4 py-3 text-xs">{statusLabels[item.status] ?? item.status}</td>
          <td className="px-4 py-3"><Button size="sm" variant="ghost" aria-expanded={expanded === item.id} onClick={() => setExpanded(expanded === item.id ? null : item.id)}>{expanded === item.id ? '收起' : '展开'}</Button></td>
        </tr>{expanded === item.id && <tr><td colSpan={8} className="border-t border-border bg-muted/20 p-4"><Report job={item} /></td></tr>}</Fragment>)}</tbody>
      </table></div>}
      <div className="flex items-center justify-between border-t border-border px-4 py-3 text-xs text-muted-foreground"><span>共 {query.data.total} 条</span><div className="flex items-center gap-2"><Button size="sm" variant="outline" aria-label="上一页真伪检测记录" disabled={query.data.page <= 1} onClick={() => setPage(query.data.page - 1)}><ChevronLeft aria-hidden="true" className="size-4" /></Button><span className="tabular-nums">{query.data.page} / {Math.max(1, Math.ceil(query.data.total / query.data.page_size))}</span><Button size="sm" variant="outline" aria-label="下一页真伪检测记录" disabled={query.data.page * query.data.page_size >= query.data.total} onClick={() => setPage(query.data.page + 1)}><ChevronRight aria-hidden="true" className="size-4" /></Button></div></div>
    </>}
  </Panel>
}
