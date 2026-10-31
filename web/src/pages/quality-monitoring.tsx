import { useState } from 'react'
import { useMutation, useQuery } from '@tanstack/react-query'
import { useSearchParams } from 'react-router-dom'
import { ChevronDown, ChevronLeft, ChevronRight, Play, Plus, ScrollText, Settings2, Trash2, TriangleAlert } from 'lucide-react'
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '../components/ui/dropdown-menu'
import { api, formatDate, formatShortDate, jsonBody, type Channel, type MonitorRun, type MonitorRunPage, type PassiveMetering, type QualityMonitor, type QualitySettings } from '../lib/api'
import { queryClient, useChannels, useQualityMonitors, useUser } from '../lib/queries'
import { Badge, Choice, Confirm, Empty, ErrorBox, Field, FormSheet, Loading, Panel, WarningTip } from '../components/common'
import { Button } from '../components/ui/button'
import { Checkbox } from '../components/ui/checkbox'
import { Input } from '../components/ui/input'
import { ModelSelect } from '../components/model-select'
import { isConcreteModel, matchesModel } from '../lib/model-rules'

const methods = [
  { value: 'integrity', label: '回答完整性', hint: '复述随机测试口令并计算随机加法，同时检查是否返回缓存的旧答案' },
  { value: 'metering', label: '计量检查', hint: '用长短两段随机文本检查用量虚报和隐藏系统提示词' },
  { value: 'tools', label: '工具调用', hint: '要求以随机参数调用一个函数，检查工具调用是否可用' },
  { value: 'truncation', label: '输出截断', hint: '要求输出几十行编号列表，检查输出上限是否被偷偷调低' },
  { value: 'streaming', label: '流式输出', hint: '以流式方式复述随机测试口令，检查事件格式、完整性和用量上报' },
]
// History also shows checks of earlier suite versions.
const checkLabels: Record<string, string> = { integrity: '回答完整性', replay: '缓存重放', metering: '计量检查', tools: '工具调用', truncation: '输出截断', streaming: '流式输出', arithmetic: '数学计算（旧版）', json: 'JSON 输出（旧版）', instruction: '指令遵循（旧版）', context: '短文本回指（旧版）' }
/** Findings that do not change the status but deserve attention in the list. */
function serviceWarnings(m: QualityMonitor) {
  return (m.last_tests ?? []).flatMap(test => {
    const d = test.detail ?? {}
    if (test.error_code) return m.last_observation === 'unknown' ? [] : [`${checkLabels[test.id] ?? test.id}未完成：${errorLabel(test.error_code)}`]
    if (test.id === 'metering' && !test.passed) return [
      ...(d.hidden_prompt ? [`疑似隐藏系统提示词（约 ${d.fixed_overhead} Token/请求）`] : []),
      ...(d.inflated ? [d.method === 'exact' ? `疑似用量虚报（上报为本地计数的 ${d.ratio} 倍）` : `疑似用量虚报（每字符 ${d.tokens_per_char} Token）`] : []),
      ...(d.method_changed ? ['计数方式已变为' + (d.method === 'exact' ? '精确比对，判定标准变严，过去的估算未必能发现这个问题' : '估算')] : []),
    ]
    if (test.id === 'tools' && !test.passed) return ['工具调用不可用']
    if (test.id === 'truncation' && !test.passed) return [`输出被截断（${d.lines}/${d.expected} 行）`]
    if (test.id === 'streaming' && !test.passed) return ['流式输出异常']
    if (test.id === 'streaming' && d.usage_reported === false) return ['流式输出没有返回用量']
    if (test.id === 'integrity' && test.passed && d.extra_text) return ['回答中有多余内容']
    return []
  })
}
const meteringReasons: Record<string, string> = {
  uncalibrated: '该模型还没有校准本地计数',
  converted_protocol: '需要转换协议，渠道支持原生接口时改用原生接口即可精确比对',
  uncountable: '请求内容无法在本地计数',
}
function checkDetail(id: string, detail: Record<string, unknown> | null | undefined) {
  if (!detail) return null
  if (id === 'metering' && detail.method === 'exact') {
    const [expected, reported] = [detail.expected as number[], detail.reported as number[]]
    return `精确比对 · 本地计数 ${Math.round(expected[0])}/${Math.round(expected[1])} Token · 上报 ${reported[0]}/${reported[1]} Token · 多出约 ${detail.fixed_overhead} Token${detail.hidden_prompt ? ' · 疑似隐藏系统提示词' : ''}${detail.inflated ? ' · 疑似用量虚报' : ''}`
  }
  if (id === 'metering') {
    const reason = meteringReasons[detail.reason as string]
    return `估算 · 每字符 ${detail.tokens_per_char} Token · 固定开销约 ${detail.fixed_overhead} Token${detail.hidden_prompt ? ' · 疑似隐藏系统提示词' : ''}${detail.inflated ? ' · 疑似用量虚报' : ' · 按比例虚报仅粗查'}${reason ? `（${reason}）` : ''}`
  }
  if (id === 'integrity' && detail.extra_text) return `回答中有约 ${detail.extra_chars} 个多余字符，可能被插入了其他内容`
  if (id === 'replay' && detail.replayed) return '第二次回答包含第一次的测试口令，疑似返回缓存答案'
  if (id === 'tools' && !detail.tool_called) return '没有返回工具调用'
  if (id === 'truncation') return `输出 ${detail.lines}/${detail.expected} 行 · 结束原因 ${detail.finish ?? '未知'}${detail.output_tokens != null ? ` · 输出 ${detail.output_tokens} Token` : ''}`
  if (id === 'streaming') return [detail.events_ok ? '事件格式正常' : '事件格式异常', detail.ended ? '正常结束' : '没有正常结束', detail.echoed ? '内容完整' : '内容不完整', detail.usage_reported ? '返回用量' : '没有返回用量'].join(' · ')
  return null
}
const errorLabels: Record<string, string> = {
  quality_probe_timeout: '上游响应超时', quality_probe_not_run: '整轮超时，未执行', quality_probe_transport: '无法连接上游',
  quality_probe_body: '读取响应失败', quality_probe_body_too_large: '响应超过 1 MB', quality_probe_invalid_json: '响应不是有效的 JSON',
  quality_probe_missing_credential: '缺少渠道凭据', quality_probe_missing_answer: '上游返回了空回答',
  quality_probe_truncated: '输出上限被推理或长回答用完，没有可评判的回答', quality_probe_reasoning_only: '只返回了推理内容，没有回答',
  quality_probe_finish_missing: '上游没有返回结束原因', quality_probe_finish_refusal: '上游安全策略拒答，本次不计入结果',
  quality_probe_finish_content_filter: '上游内容过滤拦截了回答，本次不计入结果', no_usage: '上游没有返回用量', not_streamed: '上游没有以流式返回',
}
function errorLabel(code: string) {
  if (errorLabels[code]) return errorLabels[code]
  const http = /^quality_probe_http_(\d+)$/.exec(code)
  if (http) return `上游返回 HTTP ${http[1]}`
  const finish = /^quality_probe_finish_(.+)$/.exec(code)
  if (finish) return `回答提前结束（${finish[1]}）`
  return code
}
/** Why the last check did not finish, e.g. how many checks timed out. */
function unfinishedReason(m: QualityMonitor) {
  const unfinished = (m.last_tests ?? []).filter(test => test.error_code)
  if (!unfinished.length) return null
  const timeouts = unfinished.filter(test => test.error_code === 'quality_probe_timeout' || test.error_code === 'quality_probe_not_run').length
  if (timeouts === unfinished.length) return `上游响应超时：${timeouts} 项未在 ${m.timeout_ms / 1000} 秒或整轮 ${m.round_timeout_ms / 1000} 秒内完成`
  return `${unfinished.length} 项未完成：${[...new Set(unfinished.map(test => errorLabel(test.error_code!)))].join('、')}`
}
const intervals = [{ value: '900', label: '15 分钟' }, { value: '1800', label: '30 分钟' }, { value: '3600', label: '1 小时' }, { value: '21600', label: '6 小时' }, { value: '86400', label: '24 小时' }]
const policies = [{ value: 'observe', label: '仅观察' }, { value: 'deprioritize', label: '异常时降低优先级' }, { value: 'exclude', label: '异常时剔除' }]
const defaults: QualitySettings = { schedule_enabled: false, interval_seconds: 3600, timeout_ms: 60000, round_timeout_ms: 240000, methods: methods.map(m => m.value), min_score: 75, failure_threshold: 2, daily_run_limit: 0, route_policy: 'observe' }
const activeJob = (m: QualityMonitor) => m.job_status === 'queued' || m.job_status === 'running'
const fresh = (m: QualityMonitor) => !!m.valid_until && new Date(m.valid_until).getTime() > Date.now()
function qualityState(m: QualityMonitor) {
  if (!m.channel_enabled) return { label: '渠道已停用', tone: 'neutral' as const }
  if (!m.checked_at) return { label: '尚未检测', tone: 'neutral' as const }
  if (m.last_observation === 'unknown') return { label: '检测未完成', tone: 'warning' as const }
  if (m.status === 'degraded') return { label: '确认异常', tone: 'danger' as const }
  if (m.last_observation === 'degraded') return { label: '疑似异常', tone: 'warning' as const }
  return { label: '正常', tone: 'success' as const }
}
function routingLabel(m: QualityMonitor) {
  if (m.route_policy === 'observe') return '仅观察'
  const affected = m.channel_enabled && fresh(m) && m.status === 'degraded'
  if (affected) return m.route_policy === 'exclude' ? '已剔除该模型' : '已降低该模型优先级'
  return m.route_policy === 'exclude' ? '异常时剔除' : '异常时降低优先级'
}
function settingsOf(m: QualityMonitor): QualitySettings {
  return { schedule_enabled: m.schedule_enabled, interval_seconds: m.interval_seconds, timeout_ms: m.timeout_ms, round_timeout_ms: m.round_timeout_ms, methods: m.methods, min_score: m.min_score, failure_threshold: m.failure_threshold, daily_run_limit: m.daily_run_limit, route_policy: m.route_policy }
}
function History({ monitorId, channelId, channelNames }: { monitorId?: string; channelId?: string; channelNames: Map<string, string> }) {
  const user = useUser()
  const [page, setPage] = useState(1)
  const [expanded, setExpanded] = useState<string | null>(null)
  const query = useQuery({ queryKey: ['quality-history', user.account_id, monitorId, channelId, page], queryFn: ({ signal }) => api<MonitorRunPage>(`/monitoring/runs?kind=service&page=${page}&limit=10${monitorId ? `&monitor_id=${monitorId}` : ''}${channelId ? `&channel_id=${encodeURIComponent(channelId)}` : ''}`, { signal }), refetchInterval: 5000 })
  return <Panel className="mt-5 overflow-hidden">
    <div className="border-b border-border p-4"><h2 className="text-sm font-semibold">服务体检记录</h2><p className="mt-1 text-xs text-muted-foreground">{monitorId ? '逐次查看检测方法、错误和配置版本。' : '包含已删除监控的历史。旧版渠道汇总记录保留原样。'}</p></div>
    <ErrorBox error={query.error} retry={() => void query.refetch()} />
    {query.isPending ? <Loading /> : query.data && <>
      {!query.data.items.length ? <p className="p-6 text-center text-sm text-muted-foreground">尚无检测记录，手动检测完成后将在这里显示。</p> : <div className="overflow-x-auto"><table className="w-full min-w-[650px] text-left text-sm">
        <thead className="bg-muted/30 text-xs text-muted-foreground"><tr>{['时间', '渠道 / 模型', '结果', '耗时', '详情'].map(title => <th key={title} className="px-4 py-3 font-medium">{title}</th>)}</tr></thead>
        <tbody>{query.data.items.map(run => <HistoryRow key={run.id} run={run} channelName={channelNames.get(run.channel_id) ?? '渠道'} open={expanded === run.id} toggle={() => setExpanded(expanded === run.id ? null : run.id)} />)}</tbody>
      </table></div>}
      <div className="flex items-center justify-between border-t border-border px-4 py-3 text-xs text-muted-foreground"><span>共 {query.data.total} 条</span><div className="flex items-center gap-2"><Button size="sm" variant="outline" aria-label="上一页检测记录" disabled={query.data.page <= 1} onClick={() => setPage(query.data.page - 1)}><ChevronLeft aria-hidden="true" className="size-4" /></Button><span className="tabular-nums">{query.data.page} / {Math.max(1, Math.ceil(query.data.total / 10))}</span><Button size="sm" variant="outline" aria-label="下一页检测记录" disabled={query.data.page * 10 >= query.data.total} onClick={() => setPage(query.data.page + 1)}><ChevronRight aria-hidden="true" className="size-4" /></Button></div></div>
    </>}
  </Panel>
}
function HistoryRow({ run, channelName, open, toggle }: { run: MonitorRun; channelName: string; open: boolean; toggle: () => void }) {
  return <>
    <tr className="border-t border-border">
      <td className="whitespace-nowrap px-4 py-3 text-xs tabular-nums text-muted-foreground" title={formatDate(run.finished_at)}>{formatShortDate(run.finished_at)}</td>
      <td className="max-w-56 px-4 py-3"><div>{channelName}</div><div className="break-all text-xs text-muted-foreground">{run.model}</div>{!run.monitor_id && <Badge tone="neutral">旧版渠道汇总</Badge>}</td>
      <td className="px-4 py-3">{run.kind === 'long_context'
        ? <><Badge tone={run.status === 'healthy' ? 'success' : run.status === 'degraded' ? 'danger' : 'warning'}>{run.status === 'healthy' ? '通过' : run.status === 'degraded' ? '未通过' : '未完成'}</Badge><div className="mt-1 text-xs tabular-nums text-muted-foreground">长上下文 · {run.details.max_passed ? `最大通过 ${k(run.details.max_passed)}` : '8k 档未通过'}{run.details.requested ? ` / 检测到 ${k(run.details.requested)}` : ''}</div></>
        : <><Badge tone={run.status === 'healthy' ? 'success' : run.status === 'degraded' ? 'danger' : 'warning'}>{run.status === 'healthy' ? '正常' : run.status === 'degraded' ? '异常' : '未完成'}</Badge><div className="mt-1 text-xs tabular-nums text-muted-foreground">{run.details.total ? (run.details.completed != null ? `${run.details.passed}/${run.details.completed} 项通过${run.details.completed < run.details.total ? ` · ${run.details.total - run.details.completed} 项未完成` : ''}` : `${run.details.passed}/${run.details.total} 项通过${run.score == null ? ' · 有检查未完成' : ''}`) : '—'}</div></>}</td>
      <td className="whitespace-nowrap px-4 py-3 text-xs tabular-nums">{run.latency_ms ?? '—'} ms</td>
      <td className="px-4 py-3"><Button size="sm" variant="ghost" aria-expanded={open} onClick={toggle}>{open ? '收起' : '展开'}</Button></td>
    </tr>
    {open && <tr><td colSpan={5} className="border-t border-border bg-muted/20 p-4"><div className="grid gap-3 text-xs">
      {run.kind === 'long_context' ? <p className="text-muted-foreground">长上下文检测 · 从 8k 逐档检测到 {k(run.details.requested ?? 8000)}{run.details.upstream_model ? ` · 上游模型 ${run.details.upstream_model}` : ''}</p> : <p className="text-muted-foreground">测试集 {run.details.suite_version ?? 'v1'} · 配置版本 {run.details.config_version ?? '旧版'}{run.details.upstream_model && run.details.upstream_model !== (run.details.public_model ?? run.model) ? ` · 上游模型 ${run.details.upstream_model}` : ''}</p>}
      {run.details.applied === false && <p className="text-warning">配置、渠道或监控已变化，本次结果只保留历史，不影响当前状态和路由。</p>}
      {run.error_code && <p className="break-all text-warning">{run.error_code}</p>}
      {run.details.tiers?.map(tier => <div key={tier.tokens} className="flex flex-wrap items-center gap-3"><span className="min-w-24 font-medium">{k(tier.tokens)} 档</span><Badge tone={tier.passed == null ? 'warning' : tier.passed ? 'success' : 'danger'}>{tier.passed == null ? '未完成' : tier.passed ? '找到口令' : '没有找到口令'}</Badge><span className="tabular-nums text-muted-foreground">{tier.latency_ms} ms{tier.input_tokens != null ? ` · 实际输入 ${tier.input_tokens.toLocaleString()} Token` : ''}</span>{tier.error_code && <span className="break-all text-warning">{errorLabel(tier.error_code)}</span>}</div>)}
      {run.details.tests?.map((test, i) => <div key={`${test.id}-${i}`} className="flex flex-wrap items-center gap-3"><span className="min-w-24 font-medium">{checkLabels[test.id] ?? test.id}</span><Badge tone={test.error_code ? 'warning' : test.passed ? 'success' : 'danger'}>{test.error_code ? '未完成' : test.passed ? '通过' : '未通过'}</Badge><span className="tabular-nums text-muted-foreground">{test.latency_ms} ms{test.status_code ? ` · HTTP ${test.status_code}` : ''}</span>{test.error_code && <span className="break-all text-warning">{errorLabel(test.error_code)}</span>}{checkDetail(test.id, test.detail) && <span className="w-full text-muted-foreground">{checkDetail(test.id, test.detail)}</span>}</div>)}
    </div></td></tr>}
  </>
}

/** Concrete models a channel can serve, for model pickers. */
export function channelModelOptions(channel: Channel) {
  return [...new Set([
    ...channel.known_models,
    ...channel.models,
    ...Object.keys(channel.model_mapping),
    ...(channel.test_model ? [channel.test_model] : []),
    ...(channel.source?.models ?? []),
    ...channel.discovered_models.filter(model => channel.model_mode === 'passthrough' || channel.models.some(rule => matchesModel(rule, model))),
  ])].filter(isConcreteModel).sort()
}

type Editor = { id?: string; channel_id: string; model: string; settings: QualitySettings; run_immediately: boolean }
/** Findings that do not change the status: from the last round and from real traffic. */
/** "4/5 项通过 · 1 项未完成" for the last round. */
function passedLabel(tests: { passed: boolean; error_code?: string | null }[] | null | undefined) {
  if (!tests?.length) return '—'
  const unfinished = tests.filter(t => t.error_code).length
  return `${tests.filter(t => t.passed).length}/${tests.length - unfinished} 项通过${unfinished ? ` · ${unfinished} 项未完成` : ''}`
}
/** A verdict that has expired no longer affects routing; say so when it matters. */
function staleNote(m: QualityMonitor) {
  if (!m.channel_enabled || !m.valid_until || fresh(m)) return null
  const since = `有效结果已于 ${formatShortDate(m.valid_until)} 过期`
  return m.status === 'degraded' && m.route_policy !== 'observe' ? `${since}，${m.route_policy === 'exclude' ? '剔除' : '降低优先级'}已不再生效` : since
}

function ServiceWarningTip({ monitor, traffic }: { monitor: QualityMonitor; traffic?: PassiveMetering }) {
  const warnings = serviceWarnings(monitor)
  const count = warnings.length + (traffic?.messages.length ?? 0)
  if (!count) return null
  return <WarningTip label={`${count} 项警告`}>
    {!!warnings.length && <><p className="mb-1 font-medium">最近一次服务体检有 {warnings.length} 项警告</p><ul className="grid list-disc gap-0.5 pl-4">{warnings.map(w => <li key={w}>{w}</li>)}</ul></>}
    {traffic && <><p className={`mb-1 font-medium ${warnings.length ? 'mt-2' : ''}`}>真实流量（最近 24 小时 {traffic.samples} 个请求）</p><ul className="grid list-disc gap-0.5 pl-4">{traffic.messages.map(w => <li key={w}>{w}</li>)}</ul></>}
    <p className="mt-1 text-muted-foreground">警告不改变监控状态，也不影响路由。展开检测记录可查看完整数据。</p>
  </WarningTip>
}

const k = (tokens: number) => `${Math.round(tokens / 1000)}k`

export function QualityMonitoring() {
  const user = useUser()
  const monitors = useQualityMonitors()
  const passive = useQuery({ queryKey: ['passive-metering', user.account_id], queryFn: ({ signal }) => api<PassiveMetering[]>('/passive-metering', { signal }), refetchInterval: 300_000 })
  const flagged = (passive.data ?? []).filter(p => p.warnings.length)
  const passiveFor = (m: QualityMonitor) => flagged.find(p => p.monitor_id === m.id)
  const [longFor, setLongFor] = useState<QualityMonitor | null>(null)
  const channels = useChannels()
  const [params, setParams] = useSearchParams()
  const scopedChannel = params.get('channel') ?? undefined
  const filter = params.get('quality_filter') ?? 'all'
  const search = params.get('quality_search') ?? ''
  const unmonitored = flagged.filter(p => !p.monitor_id && (!scopedChannel || p.channel_id === scopedChannel))
  const detailId = params.get('monitor') ?? undefined
  const [editor, setEditor] = useState<Editor | null>(null)
  const [remove, setRemove] = useState<QualityMonitor | null>(null)
  const [validation, setValidation] = useState<Error | null>(null)
  const [notice, setNotice] = useState('')
  const setParam = (name: string, value: string) => setParams(current => { const next = new URLSearchParams(current); if (value) next.set(name, value); else next.delete(name); return next }, { replace: true })
  const openDetail = (id: string) => setParam('monitor', id)
  const refresh = async () => { await Promise.all([queryClient.invalidateQueries({ queryKey: ['quality-monitors', user.account_id] }), queryClient.invalidateQueries({ queryKey: ['quality-history', user.account_id] })]) }
  const save = useMutation({ mutationFn: (value: Editor) => value.id ? api(`/quality-monitors/${value.id}`, jsonBody(value.settings, 'PATCH')) : api('/quality-monitors', jsonBody({ channel_id: value.channel_id, model: value.model.trim(), settings: value.settings, run_immediately: value.run_immediately, kind: 'service' })), onSuccess: async (_, value) => { setEditor(null); setNotice(!value.id && value.run_immediately ? '服务体检已添加，首次体检已排队。' : '服务体检设置已保存。'); await refresh() } })
  const run = useMutation({ mutationFn: (id: string) => api(`/quality-monitors/${id}/run`, { method: 'POST' }), onSuccess: async () => { setNotice('检测已排队，完成后将自动更新结果。'); await refresh() } })
  const deletion = useMutation({ mutationFn: (id: string) => api(`/quality-monitors/${id}`, { method: 'DELETE' }), onSuccess: async () => { setRemove(null); setNotice('监控已删除，历史记录已保留。'); await refresh() } })
  const openCreate = () => { save.reset(); setValidation(null); setEditor({ channel_id: '', model: '', settings: { ...defaults, methods: [...defaults.methods] }, run_immediately: true }) }
  // The flagged upstream model, or the public model that maps to it.
  const openCreateFor = (p: PassiveMetering) => {
    const channel = channels.data?.find(c => c.id === p.channel_id)
    const model = Object.entries(channel?.model_mapping ?? {}).find(([, upstream]) => upstream === p.upstream_model)?.[0] ?? p.upstream_model
    save.reset(); setValidation(null)
    setEditor({ channel_id: p.channel_id, model, settings: { ...defaults, methods: [...defaults.methods] }, run_immediately: channel?.enabled ?? false })
  }
  const openEdit = (m: QualityMonitor) => { save.reset(); setValidation(null); setEditor({ id: m.id, channel_id: m.channel_id, model: m.model, settings: settingsOf(m), run_immediately: false }) }
  const visible = (monitors.data ?? []).filter(m => {
    if (scopedChannel && m.channel_id !== scopedChannel) return false
    const matches = `${m.channel_name} ${m.model} ${m.upstream_model}`.toLowerCase().includes(search.toLowerCase())
    return matches && (filter === 'all' || filter === 'scheduled' && m.schedule_enabled || filter === 'manual' && !m.schedule_enabled || filter === 'attention' && (qualityState(m).tone === 'warning' || qualityState(m).tone === 'danger' || m.job_status === 'failed' || !!m.schedule_error || !!passiveFor(m)))
  })
  const channelNames = new Map(channels.data?.map(c => [c.id, c.name]))
  const detail = monitors.data?.find(m => m.id === detailId)
  const selectedChannel = channels.data?.find(c => c.id === editor?.channel_id)
  const modelOptions = selectedChannel ? channelModelOptions(selectedChannel) : []
  const setSetting = <K extends keyof QualitySettings>(key: K, value: QualitySettings[K]) => setEditor(current => current ? { ...current, settings: { ...current.settings, [key]: value } } : current)
  return <>
    <div className="mt-5 flex flex-wrap items-center justify-between gap-3"><div><h2 className="font-semibold">服务体检</h2><p className="mt-1 text-xs text-muted-foreground">每条监控对应一个渠道和模型，检查服务有没有缩水或被篡改：回答完整性、缓存重放、计量、工具调用、输出截断和流式输出，也可以手动检测长上下文。真实流量的计量异常会显示在对应监控旁。</p></div><Button onClick={openCreate}><Plus aria-hidden="true" className="size-4" />添加服务体检</Button></div>
    <div className="mt-3"><ErrorBox error={monitors.error ?? channels.error ?? run.error} retry={() => void refresh()} /></div>
    {notice && <p role="status" className="mt-3 text-sm text-primary">{notice}</p>}
    {!!unmonitored.length && <Panel className="mt-4 border-warning/40 p-4"><p className="flex items-center gap-2 text-sm font-medium text-warning"><TriangleAlert aria-hidden="true" className="size-4" />真实流量发现 {unmonitored.length} 处计量异常</p><p className="mt-1 text-xs text-muted-foreground">根据最近 24 小时的真实请求估算，不影响路由。添加服务体检后可以用主动计量检查确认。</p>
      <ul className="mt-3 grid gap-2">{unmonitored.map(p => <li key={`${p.channel_id}-${p.upstream_model}`} className="flex flex-wrap items-center gap-x-3 gap-y-1 text-sm"><span className="font-medium">{p.channel_name}</span><span className="break-all font-mono text-[13px]">{p.upstream_model}</span><span className="text-xs text-muted-foreground">{p.messages.join('；')}（{p.samples} 个请求）</span><Button size="sm" variant="outline" className="ml-auto" aria-label={`为 ${p.channel_name} / ${p.upstream_model} 添加服务体检`} onClick={() => openCreateFor(p)}>添加服务体检</Button></li>)}</ul>
    </Panel>}
    {monitors.isPending ? <Loading /> : !monitors.data ? null : !monitors.data.length ? <Panel className="mt-4"><Empty title="还没有服务体检" description="选择渠道和一个具体模型，建立独立的检测记录。默认仅观察，不影响路由。" action=<Button onClick={openCreate}>添加服务体检</Button> /></Panel> : <Panel className="mt-4 overflow-hidden">
      <div className="flex flex-wrap items-center gap-2 border-b border-border p-4"><Input name="quality_search" aria-label="搜索渠道或模型" autoComplete="off" placeholder="搜索渠道或模型…" className="w-full sm:max-w-64" value={search} onChange={e => setParam('quality_search', e.target.value)} /><div className="flex flex-wrap gap-1">{[['all', '全部'], ['attention', '需关注'], ['scheduled', '定时开启'], ['manual', '仅手动']].map(([value, label]) => <Button key={value} size="sm" variant={filter === value ? 'secondary' : 'ghost'} aria-pressed={filter === value} onClick={() => setParam('quality_filter', value)}>{label}</Button>)}</div><span className="ml-auto text-xs text-muted-foreground">{visible.length} 条监控</span></div>
      <div className="overflow-x-auto"><table className="w-full min-w-[1000px] text-left text-sm"><thead className="bg-muted/30 text-xs text-muted-foreground"><tr>{['渠道 / 模型', '最近状态', '定时监测', '最近 / 下次检测', '路由影响', '操作'].map(title => <th key={title} className={`px-4 py-3 font-medium${title === '操作' ? ' text-right' : ''}`}>{title}</th>)}</tr></thead><tbody>
        {visible.map(m => { const state = qualityState(m); return <tr key={m.id} className="border-t border-border hover:bg-muted/30">
          <td className="max-w-60 px-4 py-4"><div className="font-medium">{m.channel_name}</div><div className="mt-1 break-all text-xs">{m.model}</div>{m.model !== m.upstream_model && <div className="mt-1 break-all text-xs text-muted-foreground">→ {m.upstream_model}</div>}</td>
          <td className="px-4 py-4"><div className="flex items-center gap-1"><button type="button" className="rounded-md focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" title="查看详情" onClick={() => openDetail(m.id)}><Badge tone={state.tone}>{state.label}</Badge></button><ServiceWarningTip monitor={m} traffic={passiveFor(m)} /></div>{!!m.last_tests?.length && <div className="mt-1 text-xs tabular-nums text-muted-foreground">{passedLabel(m.last_tests)}</div>}{m.last_observation === 'unknown' && unfinishedReason(m) && <div className="mt-1 max-w-56 text-xs text-warning">{unfinishedReason(m)}</div>}{staleNote(m) && <div className="mt-1 max-w-56 text-xs text-muted-foreground">{staleNote(m)}</div>}{m.last_observation === 'degraded' && <div className="mt-1 text-xs text-muted-foreground">连续未通过 {m.consecutive_failures}/{m.failure_threshold}</div>}{activeJob(m) && <div role="status" className="mt-1 text-xs text-primary">{m.job_status === 'queued' ? '等待检测…' : '检测中…'}</div>}{(m.schedule_error || m.job_status === 'failed') && <div className="mt-1 max-w-48 text-xs text-warning">{m.schedule_error ?? '任务未完成，请重试'}</div>}</td>
          <td className="px-4 py-4"><Badge tone={m.schedule_enabled ? 'success' : 'neutral'}>{m.schedule_enabled ? '已开启' : '仅手动'}</Badge><div className="mt-1 text-xs text-muted-foreground">{!m.channel_enabled ? '渠道停用，已暂停' : m.schedule_enabled ? `每 ${intervals.find(i => Number(i.value) === m.interval_seconds)?.label ?? `${m.interval_seconds / 60} 分钟`}` : '不自动执行'}</div></td>
          <td className="whitespace-nowrap px-4 py-4 text-xs tabular-nums text-muted-foreground"><div title={m.checked_at ? formatDate(m.checked_at) : undefined}>{m.checked_at ? formatShortDate(m.checked_at) : '尚未检测'}</div><div className="mt-1">下次：{m.channel_enabled && m.schedule_enabled && m.next_run_at ? formatShortDate(m.next_run_at) : '—'}</div></td>
          <td className="px-4 py-4 text-xs">{routingLabel(m)}</td>
          <td className="px-4 py-4"><div className="flex justify-end gap-1"><div className="flex"><Button size="sm" variant="ghost" className="rounded-r-none" disabled={!m.channel_enabled || activeJob(m) || run.isPending && run.variables === m.id} onClick={() => run.mutate(m.id)}><Play aria-hidden="true" className="size-4" />体检</Button><DropdownMenu><DropdownMenuTrigger asChild><Button size="sm" variant="ghost" className="rounded-l-none px-1.5" aria-label={`更多检测 ${m.channel_name} / ${m.model}`} title="更多检测" disabled={!m.channel_enabled || activeJob(m)}><ChevronDown aria-hidden="true" className="size-4" /></Button></DropdownMenuTrigger><DropdownMenuContent align="end"><DropdownMenuItem onSelect={() => run.mutate(m.id)}><Play />立即体检</DropdownMenuItem><DropdownMenuItem onSelect={() => setLongFor(m)}><ScrollText />长上下文检测</DropdownMenuItem></DropdownMenuContent></DropdownMenu></div><Button size="sm" variant="ghost" onClick={() => openDetail(m.id)}>详情</Button><Button size="sm" variant="ghost" onClick={() => openEdit(m)}><Settings2 aria-hidden="true" className="size-4" />编辑</Button><Button size="icon" variant="ghost" aria-label={`删除 ${m.channel_name} / ${m.model} 监控`} onClick={() => { deletion.reset(); setRemove(m) }}><Trash2 aria-hidden="true" className="size-4" /></Button></div></td>
        </tr> })}
        {!visible.length && <tr><td colSpan={6} className="p-8 text-center text-muted-foreground">没有匹配的监控，试试调整筛选条件。</td></tr>}
      </tbody></table></div>
    </Panel>}
    <History key={scopedChannel ?? 'all'} channelId={scopedChannel} channelNames={channelNames} />
    <FormSheet open={!!detailId} onOpenChange={open => { if (!open) setParam('monitor', '') }} title="服务体检详情" description={detail ? `${detail.channel_name} / ${detail.model}` : '检测历史'}>
      {detail && <div className="grid gap-3 text-sm"><div className="flex flex-wrap items-center gap-3"><div className="flex items-center gap-1"><Badge tone={qualityState(detail).tone}>{qualityState(detail).label}</Badge><ServiceWarningTip monitor={detail} traffic={passiveFor(detail)} /></div><span className="text-xs text-muted-foreground">路由影响：<span className="text-foreground">{routingLabel(detail)}</span></span><Button size="sm" variant="outline" className="ml-auto" disabled={!detail.channel_enabled || activeJob(detail) || run.isPending} onClick={() => run.mutate(detail.id)}><Play aria-hidden="true" className="size-4" />{activeJob(detail) ? '体检中…' : '立即体检'}</Button><Button size="sm" variant="outline" disabled={!detail.channel_enabled || activeJob(detail)} onClick={() => setLongFor(detail)}><ScrollText aria-hidden="true" className="size-4" />长上下文检测</Button></div><p className="text-xs text-muted-foreground">路由依据有效至：{detail.valid_until ? formatDate(detail.valid_until) : '尚无有效结果'}</p><p className="text-xs text-muted-foreground">配置版本 {detail.config_version} · 每日检测次数上限：{detail.daily_run_limit || '不额外限制'}</p>{detail.last_error && <p className="break-all text-xs text-warning">{detail.last_error}</p>}</div>}
      {detail && <LongContextSummary monitor={detail} />}
      {detailId && <History key={detailId} monitorId={detailId} channelNames={channelNames} />}
    </FormSheet>
    <FormSheet open={!!longFor} onOpenChange={open => { if (!open) setLongFor(null) }} title="长上下文检测" description={longFor ? `${longFor.channel_name} / ${longFor.model}` : ''}>
      {longFor && <LongContextForm key={longFor.id} monitor={longFor} onCancel={() => setLongFor(null)} onStarted={async () => { setLongFor(null); setNotice('长上下文检测已排队，完成后结果显示在详情和检测记录中。'); await refresh() }} />}
    </FormSheet>
    <FormSheet open={!!editor} onOpenChange={open => { if (!open && !save.isPending) setEditor(null) }} title={editor?.id ? '编辑服务体检' : '添加服务体检'} description="一条监控对应一个渠道和模型。只保存结构化结果，不保存模型回答原文。">
      {editor && <form className="grid gap-5" onSubmit={e => { e.preventDefault(); setValidation(null); if (!editor.channel_id || !editor.model.trim() || !editor.settings.methods.length) { setValidation(new Error('请选择渠道、具体模型和至少一种检测方法。')); return }; save.mutate(editor) }}>
        {editor.id ? <MonitorTarget channel={channelNames.get(editor.channel_id) ?? '渠道'} channelEnabled={selectedChannel?.enabled ?? true} model={editor.model} upstream={monitors.data?.find(m => m.id === editor.id)?.upstream_model} /> : <>
          <Field label="渠道">{id => <Choice id={id} name="channel_id" value={editor.channel_id} options={(channels.data ?? []).map(c => ({ value: c.id, label: `${c.name}${c.enabled ? '' : '（已停用）'}` }))} onChange={value => setEditor({ ...editor, channel_id: value, model: '', run_immediately: channels.data?.find(c => c.id === value)?.enabled ?? false })} />}</Field>
          <Field label="模型" help="选择或输入该渠道支持的具体模型；每条监控只检测一个模型。">{id => <ModelSelect key={editor.channel_id} id={id} name="model" required disabled={!selectedChannel} value={editor.model} onChange={model => setEditor({ ...editor, model })} models={modelOptions} />}</Field>
          {selectedChannel && editor.model && <ModelMapping channel={selectedChannel} model={editor.model.trim()} />}
          {selectedChannel && !selectedChannel.enabled && <p className="text-xs text-warning">渠道已停用，监控会保留配置，启用渠道后才能执行检测。</p>}
          {selectedChannel?.kind === 'vair_key' && <p className="rounded-md bg-warning/10 px-3 py-2 text-xs leading-5 text-warning">这是 VAIR Key 渠道。每次检测会按所选方法发出多条请求，每条都通过来源 Key 发出，计入它的请求频率、并发和费用上限，费用由 Key 的所有者承担。</p>}
        </>}
        <fieldset className="grid gap-2"><legend className="mb-2 text-sm font-medium">服务体检项目</legend><div className="grid gap-2 rounded-lg border border-border p-3">{methods.map(method => <label key={method.value} className="flex min-h-11 items-start gap-3 text-sm"><Checkbox className="mt-0.5" checked={editor.settings.methods.includes(method.value)} onCheckedChange={checked => setSetting('methods', checked ? [...editor.settings.methods, method.value] : editor.settings.methods.filter(m => m !== method.value))} /><span className="grid gap-0.5"><span>{method.label}</span><span className="text-xs text-muted-foreground">{method.hint}</span></span></label>)}</div><p className="text-xs leading-5 text-muted-foreground">每次检测的参数都随机生成。回答完整性或缓存重放未通过会直接判为异常；其他项目按通过比例判定。</p></fieldset>
        <div className="grid gap-4 rounded-lg border border-border p-4"><label className="flex min-h-11 items-center gap-3 text-sm font-medium"><Checkbox checked={editor.settings.schedule_enabled} onCheckedChange={checked => setSetting('schedule_enabled', checked === true)} />开启定时监测</label><p className="text-xs text-muted-foreground">关闭后不再自动发起检测，仍可随时手动检测。</p>{editor.settings.schedule_enabled && <Field label="检测频率">{id => <Choice id={id} name="interval_seconds" value={String(editor.settings.interval_seconds)} options={intervals} onChange={value => setSetting('interval_seconds', Number(value))} />}</Field>}</div>
        <Field label="路由影响" hint="降低优先级：排在正常渠道之后，正常渠道都不可用时仍会使用。仅在连续失败达到阈值且结果仍有效时执行，结果超过有效期后恢复正常参与路由。">{id => <Choice id={id} name="route_policy" value={editor.settings.route_policy} options={policies} onChange={value => setSetting('route_policy', value as QualitySettings['route_policy'])} />}</Field>
        <p className="text-xs leading-5 text-muted-foreground">完整检测结果有效期为 {editor.settings.interval_seconds / 1800} 小时。{!editor.settings.schedule_enabled && editor.settings.route_policy !== 'observe' ? '定时已关闭：手动检测的结果到期后，降低优先级或剔除会解除。' : '新监控和检测未完成时不会自动剔除模型。'}</p>
        <details className="rounded-lg border border-border p-4"><summary className="cursor-pointer text-sm font-medium focus-visible:outline-2 focus-visible:outline-ring">高级设置</summary><div className="mt-4 grid gap-4">
          <Field label="单题超时（毫秒）" hint="10000–180000">{id => <Input id={id} name="timeout_ms" type="number" required min={10000} max={180000} value={editor.settings.timeout_ms} onChange={e => setSetting('timeout_ms', Number(e.target.value))} />}</Field>
          <Field label="整轮时限（毫秒）" hint="包含本轮所有检测方法，不小于单题超时，最大 720000。">{id => <Input id={id} name="round_timeout_ms" type="number" required min={editor.settings.timeout_ms} max={720000} value={editor.settings.round_timeout_ms} onChange={e => setSetting('round_timeout_ms', Number(e.target.value))} />}</Field>
          <Field label="最低通过比例（%）" hint="通过的检查项占比低于此值即判为未通过（回答完整性另含缓存重放一项）。">{id => <Input id={id} name="min_score" type="number" required min={0} max={100} step="0.01" value={editor.settings.min_score} onChange={e => setSetting('min_score', Number(e.target.value))} />}</Field>
          <Field label="失败确认次数" hint="网络错误不计为答题失败；一次完整通过会恢复正常。">{id => <Input id={id} name="failure_threshold" type="number" required min={1} max={5} value={editor.settings.failure_threshold} onChange={e => setSetting('failure_threshold', Number(e.target.value))} />}</Field>
          <Field label="每日检测次数预算" hint="0 表示不额外限制。按 UTC 自然日统计所有已排队轮次，包含手动检测、失败和取消的任务；每轮最多请求所选方法数次。">{id => <Input id={id} name="daily_run_limit" type="number" required min={0} max={10000} value={editor.settings.daily_run_limit} onChange={e => setSetting('daily_run_limit', Number(e.target.value))} />}</Field>
        </div></details>
        {!editor.id && <label className="flex min-h-11 items-center gap-3 text-sm"><Checkbox checked={editor.run_immediately} disabled={!selectedChannel?.enabled} onCheckedChange={checked => setEditor({ ...editor, run_immediately: checked === true })} />保存后立即检测</label>}
        <ErrorBox error={validation ?? save.error} />
        <div className="sticky bottom-0 flex justify-end gap-2 border-t border-border bg-background py-4"><Button type="button" variant="outline" disabled={save.isPending} onClick={() => setEditor(null)}>取消</Button><Button type="submit" disabled={save.isPending}>{save.isPending ? '保存中…' : editor.id ? '保存设置' : '添加服务体检'}</Button></div>
      </form>}
    </FormSheet>
    <Confirm open={!!remove} onOpenChange={open => { if (!open && !deletion.isPending) setRemove(null) }} title="删除服务体检" description={`删除 ${remove?.channel_name ?? ''} / ${remove?.model ?? ''} 的监控？后续检测和该监控的路由影响将停止，历史记录保留。`} pending={deletion.isPending} error={deletion.error} onConfirm={() => { if (remove) deletion.mutate(remove.id) }} />
  </>
}
/** The fixed channel and model of a monitor being edited. */
export function MonitorTarget({ channel, channelEnabled, model, upstream }: { channel: string; channelEnabled: boolean; model: string; upstream?: string | null }) {
  return <div className="rounded-lg border border-border bg-muted/20 p-4">
    <dl className="grid grid-cols-[3rem_minmax(0,1fr)] items-baseline gap-x-3 gap-y-2 text-sm">
      <dt className="text-xs text-muted-foreground">渠道</dt><dd className="break-all font-medium">{channel}{!channelEnabled && <span className="ml-2 text-xs font-normal text-warning">已停用</span>}</dd>
      <dt className="text-xs text-muted-foreground">模型</dt><dd className="break-all font-mono text-[13px]">{model}</dd>
      {upstream && upstream !== model && <><dt className="text-xs text-muted-foreground">上游</dt><dd className="break-all font-mono text-[13px] text-muted-foreground">{upstream}</dd></>}
    </dl>
    <p className="mt-3 border-t border-border pt-3 text-xs text-muted-foreground">渠道和模型固定，更换检测对象请新建监控。</p>
  </div>
}

function LongContextSummary({ monitor }: { monitor: QualityMonitor }) {
  const checked = monitor.long_context_checked_at
  return <section className="mt-4 grid gap-2 rounded-lg border border-border p-4 text-sm">
    <h3 className="font-medium">长上下文</h3>
    <p className="text-xs leading-5 text-muted-foreground">{checked ? `${monitor.long_context_max ? `最大通过 ${k(monitor.long_context_max)}` : '8k 档未通过'} · ${formatShortDate(checked)}` : '尚未检测。在长文本的随机位置藏一个口令，逐档检查渠道是否真的读取了完整输入；可在上方或列表的「体检」菜单中发起。'}</p>
  </section>
}

type LongContextOptions = { tiers: { tokens: number; cost_usd: number | null }[]; upstream_model: string; acknowledged: boolean; active: string | null; max_passed: number | null; checked_at: string | null }

/** Starts a long-context check; it never changes the service status or routing. */
function LongContextForm({ monitor, onCancel, onStarted }: { monitor: QualityMonitor; onCancel: () => void; onStarted: () => Promise<void> }) {
  const user = useUser()
  const options = useQuery({ queryKey: ['long-context', user.account_id, monitor.id], queryFn: ({ signal }) => api<LongContextOptions>(`/quality-monitors/${monitor.id}/long-context`, { signal }) })
  const [tier, setTier] = useState(32000)
  const [acknowledged, setAcknowledged] = useState(false)
  const start = useMutation({ mutationFn: () => api(`/quality-monitors/${monitor.id}/long-context`, jsonBody({ max_tokens: tier, acknowledge_risk: acknowledged })), onSuccess: onStarted })
  if (options.isPending) return <Loading />
  if (!options.data) return <ErrorBox error={options.error} retry={() => void options.refetch()} />
  const data = options.data
  const valid = data.acknowledged || acknowledged
  return <form className="grid gap-5" onSubmit={e => { e.preventDefault(); if (valid) start.mutate() }}>
    <fieldset className="grid gap-2"><legend className="mb-2 text-sm font-medium">检测到哪一档</legend>
      {data.tiers.map(t => <label key={t.tokens} className={`flex min-h-11 cursor-pointer items-start gap-3 rounded-lg border p-3 text-sm ${tier === t.tokens ? 'border-primary/50 bg-primary/5' : 'border-border'}`}>
        <input type="radio" name="tier" className="mt-1" checked={tier === t.tokens} onChange={() => setTier(t.tokens)} />
        <span className="grid gap-1"><span className="font-medium">{k(t.tokens)} Token</span><span className="text-xs text-muted-foreground">{t.cost_usd != null ? `按官方价格估算最多 $${t.cost_usd.toFixed(2)}` : '该模型暂无价格，无法估算'}</span></span>
      </label>)}
    </fieldset>
    <p className="text-xs leading-5 text-muted-foreground">从 8k 开始逐档检测，某一档没有找到口令就停止。费用按渠道自身计费；结果只记录，不改变服务体检的状态和路由。</p>
    {!data.acknowledged && <div className="grid gap-3 rounded-lg border border-warning/40 bg-warning/5 p-4 text-sm">
      <p className="flex items-center gap-2 font-medium text-warning"><TriangleAlert aria-hidden="true" className="size-4" />首次检测前请确认</p>
      <ul className="grid list-disc gap-1 pl-5 text-xs leading-5 text-muted-foreground"><li>检测请求可能被中转站识别；部分中转站可能因此对该 Key 区别对待，甚至限制或封禁该 Key。</li><li>长上下文检测单次输入可达十几万 Token，费用按该渠道的计费由你承担。</li><li>VAIR Key 渠道的检测请求计入来源 Key 的额度。</li></ul>
      <label className="flex min-h-11 items-center gap-3 text-sm"><Checkbox checked={acknowledged} onCheckedChange={checked => setAcknowledged(checked === true)} />我已了解以上风险</label>
    </div>}
    <ErrorBox error={start.error} />
    <div className="flex justify-end gap-2"><Button type="button" variant="outline" disabled={start.isPending} onClick={onCancel}>取消</Button><Button type="submit" disabled={!valid || start.isPending || !!data.active}>{data.active ? '已有检测在进行' : start.isPending ? '提交中…' : '开始检测'}</Button></div>
  </form>
}

export function ModelMapping({ channel, model }: { channel: Channel; model: string }) {
  const upstreamModel = channel.model_mapping[model]
  if (!upstreamModel || upstreamModel === model) return null
  return <p className="break-all text-xs text-muted-foreground">公开模型 {model} → 上游模型 {upstreamModel}</p>
}
