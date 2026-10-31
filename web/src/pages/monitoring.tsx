import { useEffect, useMemo, useState, type KeyboardEvent } from 'react'
import { useMutation, useQuery } from '@tanstack/react-query'
import { Activity, BarChart3, CheckCircle2, ChevronLeft, ChevronRight, Clock3, Radar, Settings2, ShieldAlert } from 'lucide-react'
import { api, formatDate, formatShortDate, jsonBody, type MonitorRun, type MonitorRunPage, type MonitoringChannel } from '../lib/api'
import { queryClient, useAuthenticityMonitors, useMonitoringSummary, useQualityMonitors, useUser } from '../lib/queries'
import { Button } from '../components/ui/button'
import { Checkbox } from '../components/ui/checkbox'
import { Input } from '../components/ui/input'
import { Badge, Choice, Empty, ErrorBox, Field, FormSheet, Health, Loading, PageHeader, Panel, StatCard, StatGrid } from '../components/common'
import { QualityMonitoring } from './quality-monitoring'
import { IdentityMonitoring } from './identity-monitoring'
import { Link, useSearchParams } from 'react-router-dom'
import { CLOUD } from '../lib/edition'

const availabilityRangeOptions = [
  { value: '300', label: '5 分钟' },
  { value: '900', label: '15 分钟' },
  { value: '1800', label: '30 分钟' },
  { value: '3600', label: '1 小时' },
  { value: '21600', label: '6 小时' },
  { value: '86400', label: '24 小时' },
]
const thresholdOptions = [1, 2, 3, 4, 5].map(n => ({ value: String(n), label: `${n} 次` }))
const routePolicyOptions = [
  { value: 'exclude', label: '移到路由最后（其他渠道都不可用时仍会尝试）' },
  { value: 'observe', label: '仅记录和告警，不影响路由' },
]
type FormState = { availability_enabled: boolean; availability_interval_seconds: string; availability_failure_threshold: string; availability_route_policy: string; first_token_timeout_seconds: string }
function qualityLabel(status: string) {
  return ({ healthy: '测试通过', degraded: '低于阈值', unknown: '待确认', disabled: '未启用' } as Record<string, string>)[status] ?? status
}
function runStatusLabel(run: MonitorRun) {
  if (run.kind === 'availability') return run.status === 'healthy' ? '可用' : run.status === 'unhealthy' ? '不可用' : run.status === 'rate_limited' ? '限流' : run.status === 'model_unavailable' ? '模型无效' : '待确认'
  return qualityLabel(run.status === 'unhealthy' ? 'unknown' : run.status)
}
const observationLabels = { rate_limited: '限流', model_unavailable: '模型无效', unknown: '未发出' }
/** Failure count and last error, shown in the tip beside the status badge. */
function availabilityDetail(channel: MonitoringChannel) {
  const failures = channel.availability_failures > 0 && (channel.availability_status === 'unhealthy' ? `连续失败 ${channel.availability_failures} 次${channel.availability_route_policy === 'exclude' ? '，已移到路由最后' : ''}` : `失败 ${channel.availability_failures}/${channel.availability_failure_threshold}`)
  // A failure below the threshold still says what went wrong.
  const error = (channel.availability_status !== 'healthy' || channel.availability_failures > 0) && channel.availability_error
  // An unavailable channel keeps its verdict through checks that did not conclude.
  const observation = channel.availability_status === 'unhealthy' && channel.availability_observation && `最近检查：${observationLabels[channel.availability_observation]}`
  return failures || observation || error ? <>{failures && <p>{failures}</p>}{observation && <p>{observation}</p>}{error && <p className="break-all">{error}</p>}</> : null
}
function runTone(run: MonitorRun) {
  if (run.status === 'healthy') return 'bg-success/80'
  if (run.status === 'unhealthy' || run.status === 'degraded') return 'bg-destructive/80'
  return 'bg-warning/80'
}
function Heat({ runs }: { runs: MonitorRun[] }) {
  const cells = runs.slice(0, 12).reverse()
  return <div className="flex min-w-[168px] gap-1" aria-label="最近监控状态">{cells.length ? cells.map(run => <span key={run.id} title={`${formatDate(run.finished_at)} · ${run.kind === 'availability' ? '可用性' : '质量'} · ${runStatusLabel(run)}`} className={`h-6 flex-1 rounded-sm ${runTone(run)}`} />) : Array.from({ length: 12 }, (_, index) => <span key={index} className="h-6 flex-1 rounded-sm bg-muted" />)}</div>
}

type MonitorTab = 'availability' | 'quality' | 'identity'
const runKey = (channelId: string, kind: MonitorTab | MonitorRun['kind']) => `${channelId}:${kind}`

function RecentRuns({ kind, runs, total, page, pageSize, channelNames, onPageChange }: { kind: MonitorTab; runs: MonitorRun[]; total: number; page: number; pageSize: number; channelNames: Map<string, string>; onPageChange: (page: number) => void }) {
  const pageCount = Math.max(1, Math.ceil(total / pageSize))
  const quality = kind === 'quality'
  return <Panel className="mt-5 overflow-hidden">
    <div className="flex items-center justify-between border-b border-border px-5 py-4"><div><h2 className="text-sm font-semibold">{quality ? '质量检测记录' : '可用性检查记录'}</h2><p className="mt-1 text-xs text-muted-foreground">{quality ? '每次服务体检通过的检查项和结果，不保存模型回答原文。' : '后台定时检查和手动检查的连通结果。'}</p></div><span className="text-xs text-muted-foreground">共 {total} 条</span></div>
    {runs.length ? <>
      <div className="overflow-x-auto"><table className="w-full min-w-[720px] text-left text-sm">
        <thead className="bg-muted/30 text-xs text-muted-foreground"><tr><th className="px-5 py-3 font-medium">时间</th><th className="px-5 py-3 font-medium">渠道</th><th className="px-5 py-3 font-medium">结果</th>{quality && <th className="px-5 py-3 font-medium">通过项</th>}<th className="px-5 py-3 font-medium">模型</th><th className="px-5 py-3 font-medium">耗时</th><th className="px-5 py-3 font-medium">错误</th></tr></thead>
        <tbody>{runs.map(run => <tr key={run.id} className="border-t border-border">
          <td className="whitespace-nowrap px-5 py-3 text-xs tabular-nums text-muted-foreground" title={formatDate(run.finished_at)}>{formatShortDate(run.finished_at)}</td>
          <td className="px-5 py-3 font-medium">{channelNames.get(run.channel_id) ?? '未知渠道'}</td>
          <td className="px-5 py-3"><Badge tone={run.status === 'healthy' ? 'success' : run.status === 'unhealthy' || run.status === 'degraded' ? 'danger' : 'warning'}><span className="size-1.5 rounded-full bg-current" />{runStatusLabel(run)}</Badge></td>
          {quality && <td className="px-5 py-3 text-xs tabular-nums text-muted-foreground">{run.details.total ? `${run.details.passed}/${run.details.total} 项通过` : '—'}</td>}
          <td className="max-w-[180px] truncate px-5 py-3 text-xs text-muted-foreground">{run.model ?? '—'}</td>
          <td className="whitespace-nowrap px-5 py-3 text-xs tabular-nums text-muted-foreground">{run.latency_ms === null ? '—' : `${run.latency_ms} ms`}</td>
          <td className="max-w-[220px] truncate px-5 py-3 text-xs text-muted-foreground" title={run.error_code ?? undefined}>{run.error_code ?? '—'}</td>
        </tr>)}</tbody>
      </table></div>
      <div className="flex items-center justify-between border-t border-border px-5 py-3 text-xs text-muted-foreground"><span>显示 {(page - 1) * pageSize + 1}–{Math.min(page * pageSize, total)} 条</span><div className="flex items-center gap-2"><Button size="sm" variant="outline" aria-label="上一页" onClick={() => onPageChange(page - 1)} disabled={page <= 1}><ChevronLeft className="size-4" /></Button><span>{page} / {pageCount}</span><Button size="sm" variant="outline" aria-label="下一页" onClick={() => onPageChange(page + 1)} disabled={page >= pageCount}><ChevronRight className="size-4" /></Button></div></div>
    </> : <div className="px-5 py-8 text-center text-sm text-muted-foreground">{quality ? '尚无质量检测记录。在渠道设置中开启降智检测后，记录会显示在这里。' : '尚无可用性检查记录。点击“立即检查”或等待后台检查后，记录会显示在这里。'}</div>}
  </Panel>
}

// Authenticity checks exist only on the hosted service.
const tabs: { value: MonitorTab; label: string }[] = [{ value: 'availability', label: '可用性监控' }, { value: 'quality', label: '服务体检' }, ...(CLOUD ? [{ value: 'identity' as const, label: '真伪检测' }] : [])]

function MonitorTabs({ value, onChange, badges }: { value: MonitorTab; onChange: (tab: MonitorTab) => void; badges: Record<MonitorTab, string> }) {
  // Roving tabindex with arrow / Home / End keys, per the WAI-ARIA tabs pattern.
  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    const index = tabs.findIndex(tab => tab.value === value)
    const next = event.key === 'ArrowRight' ? (index + 1) % tabs.length : event.key === 'ArrowLeft' ? (index - 1 + tabs.length) % tabs.length : event.key === 'Home' ? 0 : event.key === 'End' ? tabs.length - 1 : -1
    if (next < 0) return
    event.preventDefault()
    onChange(tabs[next].value)
    document.getElementById(`monitor-tab-${tabs[next].value}`)?.focus()
  }
  return <div role="tablist" aria-label="监控类型" className={`mt-5 grid ${tabs.length === 3 ? 'grid-cols-3' : 'grid-cols-2'} rounded-xl border border-border bg-card p-1`}>
    {tabs.map(tab => <button key={tab.value} id={`monitor-tab-${tab.value}`} type="button" role="tab" aria-selected={value === tab.value} aria-controls={`monitor-panel-${tab.value}`} tabIndex={value === tab.value ? 0 : -1} onKeyDown={onKeyDown} onClick={() => onChange(tab.value)} className={`rounded-lg px-4 py-3 text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring ${value === tab.value ? 'bg-primary/12 text-primary' : 'text-muted-foreground hover:bg-muted/40 hover:text-foreground'}`}>{tab.label} <span className="ml-1 text-xs opacity-70">{badges[tab.value]}</span></button>)}
  </div>
}

function FilterBar<T extends string>({ value, onChange, options, count }: { value: T; onChange: (value: T) => void; options: readonly (readonly [T, string])[]; count: number }) {
  return <div className="flex flex-wrap items-center gap-2 border-b border-border px-5 py-4"><div className="flex flex-wrap gap-1">{options.map(([option, label]) => <Button key={option} size="sm" variant={value === option ? 'secondary' : 'ghost'} aria-pressed={value === option} onClick={() => onChange(option)}>{label}</Button>)}</div><span className="ml-auto text-xs text-muted-foreground">{count} 个渠道</span></div>
}

function UsageLink({ channelId }: { channelId: string }) {
  return <Button asChild size="sm" variant="ghost"><Link to={`/usage?channel=${channelId}`} title="查看该渠道的真实请求用量"><BarChart3 aria-hidden="true" className="size-4" />用量</Link></Button>
}

type RowActions = { pending: Set<string>; onRun: (channelId: string) => void; onEdit: (channel: MonitoringChannel) => void }

function AvailabilityTable({ channels, runs, pending, onRun, onEdit }: { channels: MonitoringChannel[]; runs: Map<string, MonitorRun[]> } & RowActions) {
  return <div className="overflow-x-auto"><table className="w-full min-w-[880px] text-left text-sm">
    <thead className="bg-muted/30 text-xs text-muted-foreground"><tr><th className="px-5 py-3 font-medium">渠道</th><th className="px-5 py-3 font-medium">测试模型</th><th className="px-5 py-3 font-medium">状态</th><th className="px-5 py-3 font-medium">最近 12 次</th><th className="px-5 py-3 font-medium">探测首字</th><th className="px-5 py-3 font-medium">最近检查</th><th className="px-5 py-3 text-right font-medium">操作</th></tr></thead>
    <tbody>{channels.map(channel => {
      const channelRuns = runs.get(runKey(channel.id, 'availability')) ?? []
      const checking = pending.has(channel.id)
      return <tr key={channel.id} className="border-t border-border hover:bg-muted/30">
        <td className="px-5 py-4"><div className="font-medium">{channel.name}</div><div className="mt-1 text-xs text-muted-foreground">{channel.kind === 'vair_key' ? 'VAIR Key · 全部协议' : channel.protocol}</div></td>
        <td className="px-5 py-4 text-xs text-muted-foreground">{channel.test_model ?? channel.models[0] ?? (channel.kind === 'vair_key' ? '来源 Key 的第一个模型' : '未配置测试模型')}</td>
        <td className="px-5 py-4"><Health enabled={channel.enabled} monitored={channel.availability_enabled} status={channel.availability_status} errorKind={channel.availability_error_kind} detail={availabilityDetail(channel)} note={channel.availability_note} /></td>
        <td className="px-5 py-4"><Heat runs={channelRuns} /></td>
        <td className="px-5 py-4 text-xs tabular-nums text-muted-foreground">{channelRuns[0]?.ttfb_ms == null ? '—' : `${channelRuns[0].ttfb_ms} ms`}</td>
        <td className="whitespace-nowrap px-5 py-4 text-xs tabular-nums text-muted-foreground" title={channel.availability_checked_at ? formatDate(channel.availability_checked_at) : undefined}>{channel.availability_checked_at ? formatShortDate(channel.availability_checked_at) : '尚未检查'}</td>
        <td className="px-5 py-4"><div className="flex justify-end gap-1">
          <Button size="sm" variant="ghost" disabled={checking} onClick={() => onRun(channel.id)}><Activity aria-hidden="true" className="size-4" />{checking ? '检查中…' : '立即检查'}</Button>
          <UsageLink channelId={channel.id} />
          <Button size="sm" variant="ghost" onClick={() => onEdit(channel)}><Settings2 aria-hidden="true" className="size-4" />设置</Button>
        </div></td>
      </tr>
    })}</tbody>
  </table></div>
}

/** Tracks in-flight requests per channel so one row's action does not disable every row. */
function usePerChannel<T>(request: (channelId: string) => Promise<T>, onSettled: () => Promise<void>) {
  const [pending, setPending] = useState<Set<string>>(() => new Set())
  const [error, setError] = useState<Error | null>(null)
  const run = async (channelId: string) => {
    setPending(current => new Set(current).add(channelId))
    setError(null)
    try { return await request(channelId) } catch (caught) { setError(caught as Error) } finally {
      setPending(current => { const next = new Set(current); next.delete(channelId); return next })
      await onSettled()
    }
  }
  return { pending, error, run }
}

const availabilityFilters = [['all', '全部'], ['attention', '异常'], ['disabled', '未启用']] as const

export function MonitoringPage() {
  const user = useUser()
  const summary = useMonitoringSummary()
  const quality = useQualityMonitors()
  const identity = useAuthenticityMonitors()
  const [searchParams, setSearchParams] = useSearchParams()
  // Older links opened authenticity checks with ?deep= on the combined tab.
  const requested = searchParams.has('deep') ? 'identity' : searchParams.get('tab')
  const tab: MonitorTab = requested === 'quality' || (CLOUD && requested === 'identity') ? requested : 'availability'
  const scopedChannel = searchParams.get('channel')
  const runScope = scopedChannel ? `&channel_id=${encodeURIComponent(scopedChannel)}` : ''
  const [runPage, setRunPage] = useState(1)
  useEffect(() => setRunPage(1), [scopedChannel, tab])
  // Latest 12 runs per channel and kind; a global limit lets frequent availability checks crowd out quality runs.
  const heatRuns = useQuery({ queryKey: ['monitoring-runs', user.account_id, scopedChannel], queryFn: ({ signal }) => api<MonitorRun[]>(`/monitoring/runs?per_channel=12&kind=availability${runScope}`, { signal }), staleTime: 20_000, enabled: tab === 'availability' })
  const pagedRuns = useQuery({ queryKey: ['monitoring-runs-page', user.account_id, tab, runPage, scopedChannel], queryFn: ({ signal }) => api<MonitorRunPage>(`/monitoring/runs?page=${runPage}&limit=10&kind=${tab}${runScope}`, { signal }), staleTime: 20_000, enabled: tab === 'availability' })
  const [selected, setSelected] = useState<MonitoringChannel | null>(null)
  const [form, setForm] = useState<FormState | null>(null)
  const [availabilityFilter, setAvailabilityFilter] = useState<(typeof availabilityFilters)[number][0]>('all')
  const channels = useMemo(() => summary.data?.channels ?? [], [summary.data])
  const groupedRuns = useMemo(() => {
    const grouped = new Map<string, MonitorRun[]>()
    for (const run of heatRuns.data ?? []) {
      const key = runKey(run.channel_id, run.kind)
      grouped.set(key, [...(grouped.get(key) ?? []), run])
    }
    return grouped
  }, [heatRuns.data])
  const isUnhealthy = (channel: MonitoringChannel) => channel.enabled && channel.availability_enabled && channel.availability_status === 'unhealthy'
  const availabilityChannels = channels.filter(channel => {
    if (scopedChannel && channel.id !== scopedChannel) return false
    if (availabilityFilter === 'attention') return isUnhealthy(channel)
    if (availabilityFilter === 'disabled') return !channel.enabled || !channel.availability_enabled
    return true
  })
  const channelNames = useMemo(() => new Map(channels.map(channel => [channel.id, channel.name])), [channels])
  const refresh = async () => {
    await queryClient.invalidateQueries({ queryKey: ['quality-monitors', user.account_id] })
    await queryClient.invalidateQueries({ queryKey: ['monitoring-summary', user.account_id] })
    await queryClient.invalidateQueries({ queryKey: ['monitoring-runs', user.account_id, scopedChannel] })
    await queryClient.invalidateQueries({ queryKey: ['monitoring-runs-page', user.account_id] })
  }
  // The row's status, tips and recent runs show the result.
  const check = usePerChannel(channelId => api(`/channels/${channelId}/check`, { method: 'POST' }), refresh)
  const update = useMutation({
    mutationFn: ({ channelId, values }: { channelId: string; values: FormState }) => api(`/channels/${channelId}/monitor`, jsonBody({ availability_enabled: values.availability_enabled, availability_interval_seconds: Number(values.availability_interval_seconds), availability_failure_threshold: Number(values.availability_failure_threshold), availability_route_policy: values.availability_route_policy, first_token_timeout_seconds: Number(values.first_token_timeout_seconds) }, 'PATCH')),
    onSuccess: async () => { setSelected(null); setForm(null); await refresh() },
  })
  const openEditor = (channel: MonitoringChannel) => { update.reset(); setSelected(channel); setForm({ availability_enabled: channel.availability_enabled, availability_interval_seconds: String(channel.availability_interval_seconds), availability_failure_threshold: String(channel.availability_failure_threshold), availability_route_policy: channel.availability_route_policy, first_token_timeout_seconds: String(channel.first_token_timeout_seconds) }) }
  const updateForm = <K extends keyof FormState>(key: K, value: FormState[K]) => setForm(current => current ? { ...current, [key]: value } : current)
  const lastChecked = [...channels.map(c => c.availability_checked_at), ...(quality.data ?? []).map(m => m.checked_at)].filter((value): value is string => !!value).sort().at(-1)
  const qualityModelCount = quality.data?.length ?? 0
  const unhealthyCount = channels.filter(isUnhealthy).length
  const identityMismatch = identity.data?.filter(m => m.identity_verdict === 'confirmed' || m.identity_verdict === 'suspected').length ?? 0
  const degradedCount = quality.data?.filter(m => m.channel_enabled && m.status === 'degraded' && m.valid_until && new Date(m.valid_until) > new Date()).length ?? 0
  const changeTab = (next: MonitorTab) => {
    setRunPage(1)
    setSearchParams(params => { const nextParams = new URLSearchParams(params); nextParams.delete('deep'); if (next === 'availability') nextParams.delete('tab'); else nextParams.set('tab', next); return nextParams }, { replace: true })
  }
  const actions = { onEdit: openEditor }
  return <div className="monitoring-page">
    <PageHeader title="渠道监控" description={CLOUD ? '观察渠道连通状态，为每个模型添加服务体检和真伪检测。' : '观察渠道连通状态，为每个模型添加服务体检。'} action={<Button variant="outline" onClick={refresh}><Activity aria-hidden="true" className="size-4" />刷新</Button>} />
    {scopedChannel && <div className="mb-4 flex flex-wrap items-center justify-between gap-3 rounded-lg border border-border bg-card px-5 py-3"><div><p className="text-sm">当前渠道：{summary.isPending ? '加载中…' : channelNames.get(scopedChannel) ?? '渠道不存在或已删除'}</p><p className="mt-1 text-xs text-muted-foreground">渠道列表和检查记录已定位到此渠道；顶部指标仍为帐号汇总。</p></div><Button variant="outline" onClick={() => { setRunPage(1); setAvailabilityFilter('all'); setSearchParams(params => { const next = new URLSearchParams(params); next.delete('channel'); return next }, { replace: true }) }}>查看全部渠道</Button></div>}
    <ErrorBox error={summary.error ?? quality.error ?? heatRuns.error ?? pagedRuns.error ?? check.error} retry={refresh} />
    {summary.isPending ? <Loading /> : !summary.data?.channels.length ? <Panel><Empty title="还没有可监控的渠道" description="先接入一个上游渠道，再为具体模型添加模型监控。" /></Panel> : <>
      <StatGrid>
        <StatCard label="可用渠道" value={`${summary.data.available_channels}/${summary.data.total_channels}`} icon={<CheckCircle2 aria-hidden="true" />} tone={summary.data.available_channels === summary.data.total_channels ? 'success' : 'warning'} />
        <StatCard label="服务体检" value={`${qualityModelCount} 个`} detail={`覆盖 ${new Set(quality.data?.map(m => m.channel_id)).size} 个渠道`} icon={<Radar aria-hidden="true" />} />
        <StatCard label="服务异常" value={`${degradedCount}`} detail={degradedCount > 0 ? '已达确认阈值' : undefined} icon={<ShieldAlert aria-hidden="true" />} tone={degradedCount ? 'danger' : 'success'} />
        <StatCard label="最近检查" value={lastChecked ? formatShortDate(lastChecked).split(' ').at(-1) : '—'} detail={lastChecked ? formatShortDate(lastChecked).split(' ').slice(0, -1).join(' ') : '尚无探测记录'} title={lastChecked ? formatDate(lastChecked) : undefined} icon={<Clock3 aria-hidden="true" />} />
      </StatGrid>
      <MonitorTabs value={tab} onChange={changeTab} badges={{ availability: unhealthyCount ? `${unhealthyCount} 个异常` : '正常', quality: degradedCount ? `${degradedCount} 个异常` : qualityModelCount ? `${qualityModelCount} 个模型` : '待添加', identity: identityMismatch ? `${identityMismatch} 个不符` : identity.data?.length ? `${identity.data.length} 个模型` : '待添加' }} />
      <div role="tabpanel" id={`monitor-panel-${tab}`} aria-labelledby={`monitor-tab-${tab}`}>
        {tab === 'availability' ? <>
          <Panel className="mt-5 overflow-hidden">
            <FilterBar value={availabilityFilter} onChange={setAvailabilityFilter} options={availabilityFilters} count={availabilityChannels.length} />
            <AvailabilityTable channels={availabilityChannels} runs={groupedRuns} pending={check.pending} onRun={channelId => void check.run(channelId)} {...actions} />
          </Panel>
          {pagedRuns.isPending ? <Loading /> : pagedRuns.isError ? null : <RecentRuns kind={tab} runs={pagedRuns.data.items} total={pagedRuns.data.total} page={pagedRuns.data.page} pageSize={pagedRuns.data.page_size} channelNames={channelNames} onPageChange={page => setRunPage(Math.max(1, page))} />}
        </> : tab === 'quality' ? <QualityMonitoring /> : <IdentityMonitoring />}

      </div>
    </>}
    <FormSheet open={!!selected && !!form} onOpenChange={open => { if (!open && !update.isPending) { setSelected(null); setForm(null) } }} title="可用性监控设置" description={selected?.name ?? '渠道'}>
      {form && selected && <form className="grid gap-5" onSubmit={event => { event.preventDefault(); update.mutate({ channelId: selected.id, values: form }) }}>
        <label className="flex min-h-11 items-center gap-3"><Checkbox checked={form.availability_enabled} onCheckedChange={value => updateForm('availability_enabled', value === true)} />开启定时可用性检查</label>
        <Field label="检查频率">{id => <Choice id={id} value={form.availability_interval_seconds} onChange={value => updateForm('availability_interval_seconds', value)} options={availabilityRangeOptions} disabled={!form.availability_enabled} />}</Field>
        <fieldset className="grid gap-4 border-t border-border pt-4"><legend className="mb-1 text-sm font-medium">故障判定与切换</legend>
          <Field label="连续失败几次后判定异常" help="定时检查和真实请求的失败一起计数，任意一次成功即清零。有失败时下次检查提前到 10 分钟后，持续失败则逐渐放缓，最长不超过检查频率。">{id => <Choice id={id} value={form.availability_failure_threshold} onChange={value => updateForm('availability_failure_threshold', value)} options={thresholdOptions} disabled={!form.availability_enabled} />}</Field>
          <Field label="判定异常后">{id => <Choice id={id} value={form.availability_route_policy} onChange={value => updateForm('availability_route_policy', value)} options={routePolicyOptions} disabled={!form.availability_enabled} />}</Field>
          <Field label="流式首字超时（秒）" help="流式请求在这段时间内没有输出时切换到下一个渠道，客户端看不到未完成的响应。已是最后一个渠道时不切换，继续等待。5–300 秒。">{id => <Input id={id} type="number" min="5" max="300" required value={form.first_token_timeout_seconds} onChange={event => updateForm('first_token_timeout_seconds', event.target.value)} />}</Field>
        </fieldset>
        <p className="text-xs text-muted-foreground">关闭定时检查后，仍可手动检查渠道，但真实请求不再计入可用性；首字超时始终生效。{CLOUD ? '服务体检和真伪检测请在对应的 tab 中添加。' : '服务体检请在对应的 tab 中添加。'}</p>
        {selected.kind === 'vair_key' && <p className="rounded-md bg-warning/10 px-3 py-2 text-xs leading-5 text-warning">这是 VAIR Key 渠道。每次检查都通过来源 Key 发出，计入它的请求频率、并发、每日 Token 和费用上限，费用由 Key 的所有者承担。达到上限时检查会失败，但不会把渠道移出路由；来源 Key 恢复后渠道会自动可用。</p>}
        <ErrorBox error={update.error} />
        <div className="sticky bottom-0 flex justify-end gap-2 border-t border-border bg-background py-4"><Button type="button" variant="outline" disabled={update.isPending} onClick={() => { setSelected(null); setForm(null) }}>取消</Button><Button type="submit" disabled={update.isPending}>{update.isPending ? '保存中…' : '保存设置'}</Button></div>
      </form>}
    </FormSheet>
  </div>
}
