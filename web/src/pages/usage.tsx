import { useEffect, useMemo, useRef, useState } from 'react'
import { keepPreviousData, useQuery } from '@tanstack/react-query'
import { useSearchParams } from 'react-router-dom'
import { Bar, BarChart, CartesianGrid, Legend, Line, LineChart, ResponsiveContainer, Tooltip, XAxis, YAxis } from 'recharts'
import { Activity, Boxes, Clock3, Download, Filter, Info, RefreshCw, Search, Tag, XCircle, Zap } from 'lucide-react'
import { Popover, Tooltip as RadixTooltip } from 'radix-ui'
import { useChannels, useUser } from '../lib/queries'
import { api, formatDate, formatShortDate, type Protocol, type UsageEvent, type UsageEventPage, type UsageRange, type UsageSummary } from '../lib/api'
import { Badge, Choice, Empty, ErrorBox, PageHeader, Panel, StatCard, StatGrid } from '../components/common'
import { Button } from '../components/ui/button'
import { Input } from '../components/ui/input'
import { ModelPricesSheet } from '../components/model-prices'
import { UsageRequestDetails } from '../components/usage-request-details'
import { cacheMetric, successTone, usageAxisTick, usageCost } from '../lib/usage-metrics'
import { parseUsageRange, usageRanges } from '../lib/overview'

const PAGE_SIZE = 10
const EXPORT_LIMIT = 5000
type DistributionMode = 'requests' | 'tokens' | 'cost' | 'performance'

const protocolLabels: Record<Protocol, string> = { openai: 'OpenAI', anthropic: 'Anthropic', gemini: 'Gemini' }
// Only usage analysis offers all time; the overview keeps its recent windows.
const rangeOptions: { value: UsageRange; label: string }[] = [...usageRanges, { value: 'all', label: '全部时间' }]

function compact(value: number) {
  return new Intl.NumberFormat('zh-CN', { notation: 'compact', maximumFractionDigits: 1 }).format(value)
}

function whole(value: number) {
  return new Intl.NumberFormat('zh-CN').format(value)
}

function percent(part: number, total: number) {
  return total > 0 ? `${(part / total * 100).toFixed(1)}%` : '—'
}

function statusTone(status: number): 'success' | 'warning' | 'danger' {
  if (status >= 200 && status < 300) return 'success'
  if (status >= 400 && status < 500) return 'warning'
  return 'danger'
}

function statusLabel(status: number) {
  if (status >= 200 && status < 300) return '成功'
  if (status >= 400 && status < 500) return '客户端错误'
  return '服务端错误'
}

function duration(value: number) {
  return value >= 1000 ? `${(value / 1000).toFixed(1)}s` : `${whole(value)}ms`
}

function latency(value: number | null) {
  return value == null ? '—' : duration(Math.round(value))
}

function speed(value: number | null) {
  return value == null ? '—' : `${value >= 100 ? Math.round(value) : value.toFixed(1)} Token/秒`
}

function tokenTotal(row: UsageEvent) {
  return (row.input_tokens ?? 0) + (row.output_tokens ?? 0)
}

function bucketLabel(start: string, bucketSeconds: number) {
  const date = new Date(start)
  if (bucketSeconds < 86_400) return `${String(date.getHours()).padStart(2, '0')}:00`
  // 30-day buckets span years, so they carry the year instead of the day.
  if (bucketSeconds >= 30 * 86_400) return `${date.getFullYear()}/${date.getMonth() + 1}`
  return `${date.getMonth() + 1}/${date.getDate()}`
}

function bucketUnit(bucketSeconds: number) {
  if (bucketSeconds < 86_400) return '小时'
  if (bucketSeconds < 7 * 86_400) return '天'
  if (bucketSeconds < 30 * 86_400) return '周'
  return '30 天'
}

function useDebounced<T>(value: T, delay: number) {
  const [debounced, setDebounced] = useState(value)
  useEffect(() => {
    const timer = window.setTimeout(() => setDebounced(value), delay)
    return () => window.clearTimeout(timer)
  }, [value, delay])
  return debounced
}

function keyLabel(name: string | null, revoked: boolean | null) {
  return name ? `${name}${revoked ? '（已撤销）' : ''}` : ''
}

function downloadCsv(rows: UsageEvent[]) {
  const header = ['渠道', 'API Key', '请求模型', '实际上游模型', '费用(USD)', '协议', '状态', '首字(ms)', '总耗时(ms)', '输入 Token', '输出 Token', '缓存 Token', '时间']
  const body = rows.map(row => [row.channel_name ?? '', keyLabel(row.key_name, row.key_revoked), row.model, row.upstream_model ?? '', row.cost_usd ?? '', protocolLabels[row.protocol], row.status, row.ttfb_ms ?? '', row.duration_ms, row.input_tokens ?? '', row.output_tokens ?? '', row.cached_tokens ?? '', formatDate(row.created_at)])
  const csv = [header, ...body].map(line => line.map(value => `"${String(value).replaceAll('"', '""')}"`).join(',')).join('\n')
  const url = URL.createObjectURL(new Blob([`\ufeff${csv}`], { type: 'text/csv;charset=utf-8' }))
  const link = document.createElement('a')
  link.href = url
  link.download = `usage-analysis-${new Date().toISOString().slice(0, 10)}.csv`
  link.click()
  URL.revokeObjectURL(url)
}

function CellTooltip({ content, children }: { content: string | null | undefined; children: React.ReactNode }) {
  if (!content) return <>{children}</>
  return (
    <RadixTooltip.Provider delayDuration={300}>
      <RadixTooltip.Root>
        <RadixTooltip.Trigger asChild>
          <span className="block w-full cursor-default">{children}</span>
        </RadixTooltip.Trigger>
        <RadixTooltip.Portal>
          <RadixTooltip.Content side="top" align="center" sideOffset={6} className="z-50 max-w-xs rounded-md border bg-popover px-3 py-2 text-xs text-popover-foreground shadow-md"><p>{content}</p><RadixTooltip.Arrow className="fill-border" /></RadixTooltip.Content>
        </RadixTooltip.Portal>
      </RadixTooltip.Root>
    </RadixTooltip.Provider>
  )
}

function ChannelPerformance({ channels, selected, onSelect }: { channels: UsageSummary['channels']; selected: string | null; onSelect: (channelId: string | null) => void }) {
  const notes = '请求错误通常为参数或鉴权问题，服务异常为上游或网关失败；缓存率仅按已上报请求中的缓存 Token / 输入 Token 计算。'
  return <Panel className="usage-table-panel">
    <div className="usage-table-heading"><div><h2 className="flex items-center gap-1">渠道表现<Popover.Root><Popover.Trigger asChild><button type="button" aria-label="显示说明" className="inline-flex items-center justify-center rounded-sm text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"><Info className="size-4" /></button></Popover.Trigger><Popover.Portal><Popover.Content side="bottom" align="start" sideOffset={6} className="z-50 max-w-xs rounded-md border bg-popover px-3 py-2 text-xs text-popover-foreground shadow-md"><p>{notes}</p><Popover.Arrow className="fill-border" /></Popover.Content></Popover.Portal></Popover.Root></h2><p>对比各渠道的请求成功率、Token 用量与费用。</p></div><div className="flex items-center gap-2">{selected && <Button variant="ghost" size="sm" onClick={() => onSelect(null)}><XCircle />查看全部渠道</Button>}</div></div>
    {channels.length ? <div className="overflow-x-auto"><table className="usage-table usage-channel-table">
        <colgroup><col className="usage-col-channel" /><col className="usage-col-requests" /><col className="usage-col-success" /><col className="usage-col-tokens" /><col className="usage-col-cache" /><col className="usage-col-cost" /><col className="usage-col-latency" /><col className="usage-col-speed" /></colgroup>
        <thead><tr><th scope="col">渠道</th><th scope="col">请求数</th><th scope="col">成功率</th><th scope="col">Token 用量</th><th scope="col">缓存率</th><th scope="col">估算费用</th><th scope="col">首字 / 总耗时</th><th scope="col">输出速度</th></tr></thead>
        <tbody>{channels.map(channel => {
          const active = channel.channel_id !== null && channel.channel_id === selected
          const name = channel.channel_name ?? (channel.channel_id ? '已删除渠道' : '未记录渠道')
          const cache = cacheMetric(channel)
          const tone = successTone(channel.successful, channel.requests)
          const otherErrors = Math.max(0, channel.requests - channel.successful - channel.client_errors - channel.server_errors)
          const successDetail = [
            channel.requests > 0 ? `${whole(channel.successful)} / ${whole(channel.requests)} 次成功` : null,
            channel.requests > 0 && channel.requests < 20 ? '样本较少' : null,
            channel.client_errors > 0 ? `请求错误 ${whole(channel.client_errors)} 次` : null,
            channel.server_errors > 0 ? `服务异常 ${whole(channel.server_errors)} 次` : null,
            otherErrors > 0 ? `其他未成功 ${whole(otherErrors)} 次` : null,
            channel.requests === 0 ? '暂无请求' : null,
          ].filter(Boolean).join('  |  ')
          return <tr key={channel.channel_id ?? 'none'} aria-selected={active} className={active ? 'bg-primary/5' : undefined}>
            <td>{channel.channel_id && channel.channel_name ? <button type="button" className="usage-channel-name font-medium text-left hover:text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring rounded-sm" aria-pressed={active} onClick={() => onSelect(active ? null : channel.channel_id)}>{name}</button> : <span className="usage-channel-name text-muted-foreground" title="渠道字段上线前的历史请求，或渠道已删除">{name}</span>}</td>
            <td className="tabular-nums">{whole(channel.requests)}</td>
            <td>
              <CellTooltip content={successDetail}>
                <div className={`usage-success-value usage-success-${tone} tabular-nums`}><span aria-hidden="true" className="usage-success-dot" />{percent(channel.successful, channel.requests)}</div>
              </CellTooltip>
            </td>
            <td>
              <div className="font-medium tabular-nums">{compact(channel.input_tokens + channel.output_tokens)}</div>
              <div className="usage-channel-tokens"><span>输入 <span className="tabular-nums">{compact(channel.input_tokens)}</span></span><span>输出 <span className="tabular-nums">{compact(channel.output_tokens)}</span></span></div>
            </td>
            <td>
              <CellTooltip content={cache.title}>
                <div className={`usage-cache-value tabular-nums${cache.rate != null && cache.rate > 0 ? ' text-primary' : ' text-muted-foreground'}`}>{cache.value}</div>
                {cache.rate != null && <div className={`usage-cache-track usage-cache-${cache.level}`} aria-hidden="true"><span style={{ width: `${Math.min(100, Math.max(0, cache.rate))}%` }} /></div>}
              </CellTooltip>
            </td>
            <td className="tabular-nums">
              {usageCost(channel.estimated_cost_usd ?? 0)}
            </td>
            <td>
              <div className="tabular-nums" title="流式请求收到首个输出的中位数">{latency(channel.p50_first_token_ms)}</div>
              <div className="usage-token-breakdown">总耗时 {latency(channel.p50_duration_ms)}</div>
            </td>
            <td>
              <div className="whitespace-nowrap tabular-nums">{speed(channel.p50_output_tps)}</div>
              <div className="usage-token-breakdown">{channel.speed_samples ? `${whole(channel.speed_samples)} 个流式请求${channel.speed_samples < 20 ? ' · 样本较少' : ''}` : '无流式请求'}</div>
            </td>
          </tr>
        })}</tbody>
      </table></div>
     : <Empty title="暂无渠道数据" description="当前筛选条件下没有请求。" />}
  </Panel>
}
export function UsagePage() {
  const user = useUser()
  const channelList = useChannels()
  const [searchParams, setSearchParams] = useSearchParams()
  const channel = searchParams.get('channel')
  const key = searchParams.get('key')
  const rangeParam = searchParams.get('range')
  const range: UsageRange = rangeParam === 'all' ? 'all' : parseUsageRange(rangeParam)
  const model = searchParams.get('model') || 'all'
  const protocolParam = searchParams.get('protocol')
  const protocol = protocolParam && ['openai', 'anthropic', 'gemini'].includes(protocolParam) ? protocolParam : 'all'
  const statusParam = searchParams.get('status')
  const status = statusParam === 'error' || statusParam === 'success' ? statusParam : 'all'
  const search = searchParams.get('search') ?? ''
  const setFilter = (name: string, value: string) => setSearchParams(params => {
    const next = new URLSearchParams(params)
    if (value && value !== 'all') next.set(name, value); else next.delete(name)
    return next
  }, { replace: true })
  // `all` means "no filter" elsewhere, but for the range it is a real choice.
  const setRange = (value: string) => setSearchParams(params => { const next = new URLSearchParams(params); next.set('range', value); return next }, { replace: true })
  const setModel = (value: string) => setFilter('model', value)
  const setProtocol = (value: string) => setFilter('protocol', value)
  const setStatus = (value: string) => setFilter('status', value)
  const setSearch = (value: string) => setFilter('search', value)
  const pricesOpen = searchParams.get('prices') === '1'
  const priceSearch = searchParams.get('price_q') ?? ''
  const openPrices = (model?: string) => setSearchParams(params => { const next = new URLSearchParams(params); next.set('prices', '1'); if (model) next.set('price_q', model); else next.delete('price_q'); return next }, { replace: true })
  const closePrices = () => setSearchParams(params => { const next = new URLSearchParams(params); next.delete('prices'); next.delete('price_q'); return next }, { replace: true })
  const debouncedSearch = useDebounced(search.trim(), 300)
  const [selectedRequest, setSelectedRequest] = useState<string | null>(null)
  const requestTrigger = useRef<HTMLButtonElement | null>(null)
  const [page, setPage] = useState(1)
  const [distributionMode, setDistributionMode] = useState<DistributionMode>('requests')
  const [exporting, setExporting] = useState(false)
  const [exportError, setExportError] = useState<Error | null>(null)

  const filterParams = useMemo(() => {
    const params = new URLSearchParams({ range, utc_offset_minutes: String(-new Date().getTimezoneOffset()) })
    if (model !== 'all') params.set('model', model)
    if (protocol !== 'all') params.set('protocol', protocol)
    if (status !== 'all') params.set('status', status)
    if (channel) params.set('channel_id', channel)
    if (key) params.set('key_id', key)
    if (debouncedSearch) params.set('search', debouncedSearch)
    return params.toString()
  }, [channel, debouncedSearch, key, model, protocol, range, status])
  // Search changes are debounced, so reset paging when the effective filter changes rather than on each keystroke.
  useEffect(() => setPage(1), [filterParams])

  const summary = useQuery({ queryKey: ['usage-summary', user.account_id, filterParams], queryFn: ({ signal }) => api<UsageSummary>(`/usage/summary?${filterParams}`, { signal }), placeholderData: keepPreviousData })
  const events = useQuery({ queryKey: ['usage-events', user.account_id, filterParams, page, PAGE_SIZE], queryFn: ({ signal }) => api<UsageEventPage>(`/usage/events?${filterParams}&page=${page}&limit=${PAGE_SIZE}`, { signal }), placeholderData: keepPreviousData })
  const refetch = () => { void summary.refetch(); void events.refetch() }

  const totals = summary.data?.totals
  const totalCache = totals ? cacheMetric(totals) : null
  const bucketSeconds = summary.data?.bucket_seconds ?? 0
  const chartData = useMemo(() => (summary.data?.trend ?? []).map(bucket => ({ ...bucket, label: bucketLabel(bucket.start, bucketSeconds) })), [summary.data, bucketSeconds])
  const hasTrend = chartData.some(bucket => bucket.requests > 0)
  const hasPerformance = chartData.some(bucket => bucket.p50_output_tps != null || bucket.p50_first_token_ms != null)
  const hasCostTrend = chartData.some(bucket => bucket.requests > 0 && bucket.estimated_cost_usd > 0)
  const modelDistribution = useMemo(() => [...(summary.data?.models ?? [])].map(item => {
    const value = distributionMode === 'tokens' ? item.tokens : distributionMode === 'cost' ? (item.estimated_cost_usd ?? 0) : item.requests
    return [item.model, value, item.estimated_cost_usd] as const
  }).sort((a, b) => b[1] - a[1]).slice(0, 5), [distributionMode, summary.data])
  const modelOptions = useMemo(() => {
    const options = new Set(summary.data?.model_options ?? [])
    if (model !== 'all') options.add(model)
    return [...options].sort().map(value => ({ value, label: value }))
  }, [model, summary.data])
  const channelOptions = useMemo(() => {
    const options = (channelList.data ?? []).map(item => ({ value: item.id, label: item.name }))
    if (channel && !options.some(option => option.value === channel)) options.push({ value: channel, label: '未知渠道' })
    return options
  }, [channel, channelList.data])
  const keyOptions = useMemo(() => {
    const options = (summary.data?.key_options ?? []).map(item => ({ value: item.id, label: keyLabel(item.name, item.revoked) }))
    if (key && !options.some(option => option.value === key)) options.push({ value: key, label: '未知 Key' })
    return options
  }, [key, summary.data])

  const total = events.data?.total ?? 0
  const pageRows = events.data?.items ?? []
  const pageCount = Math.max(1, Math.ceil(total / PAGE_SIZE))
  const setChannel = (value: string | null) => setSearchParams(params => { const next = new URLSearchParams(params); if (value) next.set('channel', value); else next.delete('channel'); return next }, { replace: true })
  const setKey = (value: string) => setFilter('key', value)
  const resetFilters = () => setSearchParams({}, { replace: true })
  const exportCsv = async () => {
    setExporting(true)
    setExportError(null)
    try {
      const result = await api<UsageEventPage>(`/usage/events?${filterParams}&page=1&limit=${EXPORT_LIMIT}`)
      downloadCsv(result.items)
    } catch (caught) { setExportError(caught as Error) } finally { setExporting(false) }
  }
  const fetching = summary.isFetching || events.isFetching

  return <div className="usage-page">
    <PageHeader title="使用分析" description="基于真实网关请求统计趋势、Token 消耗、缓存率和各渠道表现，可按 API Key 筛选。" action={<div className="usage-header-actions"><div className="usage-header-choice"><span className="sr-only">渠道筛选</span><Choice aria-label="渠道筛选" value={channel ?? 'all'} onChange={value => setChannel(value === 'all' ? null : value)} options={[{ value: 'all', label: '全部渠道' }, ...channelOptions]} /></div><div className="usage-header-choice usage-header-choice-wide"><span className="sr-only">API Key 筛选</span><Choice aria-label="API Key 筛选" value={key ?? 'all'} onChange={setKey} options={[{ value: 'all', label: '全部 Key' }, ...keyOptions]} /></div><div className="usage-header-choice"><span className="sr-only">数据范围</span><Choice aria-label="数据范围" value={range} onChange={value => setRange(value as UsageRange)} options={rangeOptions} /></div><div className="usage-header-choice"><span className="sr-only">图表统计方式</span><Choice aria-label="图表统计方式" value={distributionMode} onChange={value => setDistributionMode(value as DistributionMode)} options={[{ value: 'requests', label: '按请求数' }, { value: 'tokens', label: '按 Token 数' }, { value: 'cost', label: '按费用' }, { value: 'performance', label: '输出性能' }]} /></div><div className="usage-header-buttons"><Button variant="outline" onClick={() => openPrices()}><Tag />模型单价</Button><Button variant="outline" onClick={refetch} disabled={fetching}><RefreshCw className={fetching ? 'animate-spin' : ''} />刷新数据</Button></div></div>} />
    <ErrorBox error={summary.error ?? events.error ?? exportError} retry={refetch} />
    {summary.isError ? null : summary.isPending || !totals ? <div className="grid gap-4 md:grid-cols-4"><div className="h-32 animate-pulse rounded-xl bg-muted md:col-span-4" /><div className="h-72 animate-pulse rounded-xl bg-muted md:col-span-3" /></div> : <>
      <StatGrid>
        <StatCard label="总请求数" value={whole(totals.requests)} detail={`${percent(totals.successful, totals.requests)} 请求成功`} icon={<Activity aria-hidden="true" />} />
        <StatCard label="总 Token" value={compact(totals.input_tokens + totals.output_tokens)} detail={<span title={totalCache?.title}>缓存率 {totalCache?.value}{totalCache?.detail ? ` · ${totalCache.detail}` : ''}</span>} icon={<Zap aria-hidden="true" />} tone="success" />
        <StatCard label="平均耗时" value={latency(totals.avg_duration_ms)} detail={`首字 ${latency(totals.avg_ttfb_ms)}`} icon={<Clock3 aria-hidden="true" />} tone="warning" />
        <StatCard label="活跃模型数" value={whole(totals.active_models)} icon={<Boxes aria-hidden="true" />} />
      </StatGrid>

      <div className="usage-analysis-grid">
        <Panel className="usage-chart-panel">
          <div className="usage-panel-heading"><div><h2>{distributionMode === 'performance' ? '输出性能' : '请求趋势'}</h2><p>按{bucketUnit(bucketSeconds)}查看{distributionMode === 'performance' ? '流式请求的首字时间和输出速度（中位数）' : distributionMode === 'requests' ? '请求量' : distributionMode === 'tokens' ? 'Token 用量（K=千，M=百万，B=十亿）' : <>调用时费用（USD），按<button type="button" className="usage-inline-link" onClick={() => openPrices()}>模型单价</button>估算</>}</p></div></div>
          {distributionMode === 'performance' ? hasPerformance ? <div className="usage-chart">
            <ResponsiveContainer width="100%" height="100%">
              <LineChart data={chartData} margin={{ top: 12, right: 8, left: 0, bottom: 0 }}>
                <CartesianGrid stroke="rgba(226,246,244,.07)" vertical={false} />
                <XAxis dataKey="label" tick={{ fill: '#9aaeb3', fontSize: 11 }} tickLine={false} axisLine={false} interval="preserveStartEnd" minTickGap={12} height={20} />
                <YAxis yAxisId="speed" width={50} tick={{ fill: '#9aaeb3', fontSize: 11 }} tickLine={false} axisLine={false} tickFormatter={value => `${Math.round(Number(value))}`} />
                <YAxis yAxisId="first" orientation="right" width={50} tick={{ fill: '#9aaeb3', fontSize: 11 }} tickLine={false} axisLine={false} tickFormatter={value => duration(Math.round(Number(value)))} />
                <Tooltip contentStyle={{ background: '#17232a', border: '1px solid rgba(226,246,244,.14)', borderRadius: 8, color: '#eef6f5' }} labelFormatter={(_, payload) => payload?.[0] ? formatDate(payload[0].payload.start) : ''} formatter={(value, name) => name === 'p50_output_tps' ? [speed(Number(value)), '输出速度'] : [latency(Number(value)), '首字时间']} />
                <Legend formatter={value => value === 'p50_output_tps' ? '输出速度（左轴，Token/秒）' : '首字时间（右轴）'} wrapperStyle={{ fontSize: 11, color: '#9aaeb3' }} />
                <Line yAxisId="speed" type="monotone" dataKey="p50_output_tps" stroke="#78d9d0" strokeWidth={2} dot={false} connectNulls />
                <Line yAxisId="first" type="monotone" dataKey="p50_first_token_ms" stroke="#e8c37b" strokeWidth={2} dot={false} connectNulls />
              </LineChart>
            </ResponsiveContainer>
          </div> : <Empty title="暂无输出性能数据" description="输出性能只统计成功的流式请求；输出少于 32 个 Token 的请求不计入速度。" />
          : hasTrend && (distributionMode !== 'cost' || hasCostTrend) ? <div className="usage-chart">
            <ResponsiveContainer width="100%" height="100%">
              <BarChart data={chartData} margin={{ top: 12, right: 50, left: 0, bottom: 0 }}>
                <CartesianGrid stroke="rgba(226,246,244,.07)" vertical={false} />
                <XAxis dataKey="label" tick={{ fill: '#9aaeb3', fontSize: 11 }} tickLine={false} axisLine={false} interval="preserveStartEnd" minTickGap={12} height={20} />
                <YAxis width={50} allowDecimals={distributionMode === 'cost'} tick={{ fill: '#9aaeb3', fontSize: 11 }} tickLine={false} axisLine={false} tickFormatter={value => usageAxisTick(Number(value), distributionMode === 'cost')} />
                <Tooltip cursor={{ fill: 'rgba(120,217,208,.06)' }} contentStyle={{ background: '#17232a', border: '1px solid rgba(226,246,244,.14)', borderRadius: 8, color: '#eef6f5' }} labelFormatter={(_, payload) => payload?.[0] ? formatDate(payload[0].payload.start) : ''} formatter={value => distributionMode === 'cost' ? [usageCost(Number(value)), '费用'] : [whole(Number(value)), distributionMode === 'requests' ? '请求数' : 'Token']} />
                <Bar dataKey={distributionMode === 'cost' ? 'estimated_cost_usd' : distributionMode} fill="#78d9d0" radius={[4, 4, 0, 0]} maxBarSize={28} />
              </BarChart>
            </ResponsiveContainer>
          </div> : <Empty title={hasTrend && distributionMode === 'cost' ? '暂无费用数据' : '暂无趋势数据'} description={hasTrend && distributionMode === 'cost' ? '所选时段没有产生费用。' : '调整时间范围或筛选条件后再试。'} />}
        </Panel>
        <Panel className="usage-distribution-panel">
          <div className="usage-panel-heading"><div><h2>模型分布</h2><p>{(distributionMode === 'requests' || distributionMode === 'performance') ? '当前筛选结果中的请求占比' : distributionMode === 'tokens' ? '当前筛选结果中的 Token 占比' : '当前筛选结果中的估算费用占比'}</p></div></div>
          {modelDistribution.length ? <div className="usage-distribution-list">{modelDistribution.map(([name, count, cost]) => <div key={name} className="usage-distribution-row"><div className="flex min-w-0 items-center justify-between gap-3"><span className="truncate text-sm">{name}</span><span className="tabular-nums text-xs text-muted-foreground">{(distributionMode === 'requests' || distributionMode === 'performance') ? `${whole(count)} 次` : distributionMode === 'tokens' ? `${compact(count)} Token` : (cost == null ? '费用未知' : usageCost(cost))}</span></div><div className="usage-distribution-track"><span style={{ width: `${count > 0 ? Math.max(8, count / (modelDistribution[0][1] || 1) * 100) : 0}%` }} /></div></div>)}</div> : <Empty title="暂无模型数据" description="当前筛选条件下没有请求。" />}
        </Panel>
      </div>

      <ChannelPerformance channels={summary.data.channels} selected={channel} onSelect={setChannel} />

      <Panel className="usage-table-panel">
        <div className="usage-table-heading"><div><h2>调用明细</h2><p>点击请求查看耗时分解与渠道尝试过程</p></div><div className="flex flex-wrap gap-2"><Button variant="outline" size="sm" onClick={exportCsv} disabled={!total || exporting} title={total > EXPORT_LIMIT ? `最多导出最近 ${whole(EXPORT_LIMIT)} 条` : undefined}><Download />{exporting ? '导出中…' : '导出 CSV'}</Button><Button variant="ghost" size="sm" onClick={resetFilters}><Filter />重置筛选</Button></div></div>
        <div className="usage-filters"><div className="usage-search-wrap"><Search aria-hidden="true" /><Input aria-label="搜索模型或状态" placeholder="搜索模型、协议或状态…" value={search} onChange={event => setSearch(event.target.value)} /></div><Choice aria-label="渠道筛选" value={channel ?? 'all'} onChange={value => setChannel(value === 'all' ? null : value)} options={[{ value: 'all', label: '全部渠道' }, ...channelOptions]} /><Choice aria-label="API Key 筛选" value={key ?? 'all'} onChange={setKey} options={[{ value: 'all', label: '全部 Key' }, ...keyOptions]} /><Choice aria-label="模型筛选" value={model} onChange={setModel} options={[{ value: 'all', label: '全部模型' }, ...modelOptions]} /><Choice aria-label="协议筛选" value={protocol} onChange={setProtocol} options={[{ value: 'all', label: '全部协议' }, ...Object.entries(protocolLabels).map(([value, label]) => ({ value, label }))]} /><Choice aria-label="请求状态" value={status} onChange={setStatus} options={[{ value: 'all', label: '全部状态' }, { value: 'success', label: '成功请求' }, { value: 'error', label: '异常请求' }]} /></div>
        {pageRows.length ? <div className="overflow-x-auto"><table className="usage-table"><thead><tr><th>模型</th><th>渠道 / API Key</th><th>协议</th><th>状态</th><th>首字</th><th>总耗时</th><th>Token 用量</th><th><span className="inline-flex items-center gap-1">费用<button type="button" className="usage-inline-icon" aria-label="查看模型单价" title="查看模型单价" onClick={() => openPrices()}><Info aria-hidden="true" /></button></span></th><th>时间</th></tr></thead><tbody>{pageRows.map(row => <tr key={row.id} className="cursor-pointer" onClick={event => { requestTrigger.current = event.currentTarget.querySelector<HTMLButtonElement>('[data-request-details]'); setSelectedRequest(row.id) }}><td><button type="button" data-request-details aria-label={`查看 ${row.model} 请求详情`} aria-haspopup="dialog" className="min-h-10 rounded-sm text-left font-medium underline-offset-4 hover:text-primary hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">{row.model}</button>{row.source === 'monitor' && <span className="ml-1.5" title="其他帐号通过 VAIR Key 渠道发起的检查或模型监控"><Badge>监控</Badge></span>}</td><td className="text-muted-foreground"><div>{row.channel_name ?? '—'}</div>{row.key_name && <div className="mt-1 text-xs" title="API Key">{keyLabel(row.key_name, row.key_revoked)}</div>}</td><td><span className="usage-protocol">{protocolLabels[row.protocol]}</span></td><td><Badge tone={statusTone(row.status)}><span className="size-1.5 rounded-full bg-current" />{row.status} · {statusLabel(row.status)}</Badge></td><td className="tabular-nums" title={row.stream ? '流式请求：收到首个输出的时间' : '非流式请求：收到响应的时间'}>{latency(row.stream && row.first_token_ms != null ? row.first_token_ms : row.ttfb_ms)}</td><td className="tabular-nums">{duration(row.duration_ms)}</td><td><div className="tabular-nums">{row.input_tokens == null && row.output_tokens == null ? '—' : compact(tokenTotal(row))}</div><div className="usage-token-breakdown">输入 {row.input_tokens == null ? '—' : compact(row.input_tokens)} · 输出 {row.output_tokens == null ? '—' : compact(row.output_tokens)}{row.cached_tokens ? ` · 缓存 ${compact(row.cached_tokens)}` : ''}</div></td><td className="tabular-nums">{row.cost_usd != null ? <span title={row.price_snapshot ? `输入 ${row.price_snapshot.input_per_1m} / 输出 ${row.price_snapshot.output_per_1m} / 缓存读 ${row.price_snapshot.cached_read_per_1m} USD/1M` : undefined}>${row.cost_usd.toFixed(6)}</span> : row.pricing_status === 'unpriced' ? <button type="button" className="usage-inline-link text-muted-foreground" title="该模型还没有单价，查看模型单价" onClick={event => { event.stopPropagation(); openPrices(row.upstream_model ?? row.model) }}>未定价</button> : <span className="text-muted-foreground">—</span>}</td><td className="whitespace-nowrap text-muted-foreground tabular-nums" title={formatDate(row.created_at)}>{formatShortDate(row.created_at)}</td></tr>)}</tbody></table></div> : events.isPending ? <div className="h-40 animate-pulse bg-muted/40" /> : <Empty title="没有匹配的请求" description="试试清空搜索词，或调整时间范围和筛选条件。" action={<Button variant="outline" size="sm" onClick={resetFilters}><XCircle />清空筛选</Button>} />}
        <div className="usage-pagination"><span>显示 {total ? (page - 1) * PAGE_SIZE + 1 : 0}–{Math.min(page * PAGE_SIZE, total)} 条，共 {whole(total)} 条</span><div className="flex items-center gap-2"><Button variant="outline" size="sm" onClick={() => setPage(current => Math.max(1, current - 1))} disabled={page <= 1}>上一页</Button><span className="usage-page-number tabular-nums">{page} / {pageCount}</span><Button variant="outline" size="sm" onClick={() => setPage(current => Math.min(pageCount, current + 1))} disabled={page >= pageCount}>下一页</Button></div></div>
      </Panel>
    </>}
    <ModelPricesSheet key={priceSearch} open={pricesOpen} initialSearch={priceSearch} onOpenChange={open => { if (open) openPrices(priceSearch); else closePrices() }} />
    <UsageRequestDetails requestId={selectedRequest} accountId={user.account_id} onClose={() => setSelectedRequest(null)} returnFocus={() => requestTrigger.current?.focus({ preventScroll: true })} />
  </div>
}
