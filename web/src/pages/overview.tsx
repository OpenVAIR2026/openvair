import { useMemo, type ReactNode } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Bar, BarChart, CartesianGrid, ResponsiveContainer, Tooltip, XAxis, YAxis } from 'recharts'
import { ArrowRight, CheckCircle2, KeyRound, Network, RefreshCw, ShieldAlert } from 'lucide-react'
import { Link, useSearchParams } from 'react-router-dom'
import { useKeys, useMonitoringSummary, useQualityMonitors, useUser } from '../lib/queries'
import { api, formatDate, formatShortDate, type Protocol, type UsageEventPage, type UsageSummary } from '../lib/api'
import { monitoringHref, overviewChannelState, parseUsageRange, usageHref, usageRanges } from '../lib/overview'
import { successTone, usageAxisTick } from '../lib/usage-metrics'
import { Badge, Choice, Empty, ErrorBox, Loading, PageHeader, Panel } from '../components/common'
import { Button } from '../components/ui/button'
import { GettingStarted, useOnboarding } from '../components/getting-started'

const protocolLabels: Record<Protocol | 'any', string> = { openai: 'OpenAI', anthropic: 'Anthropic', gemini: 'Gemini', any: '全部协议' }
const whole = (value: number) => new Intl.NumberFormat('zh-CN').format(value)
const compact = (value: number) => new Intl.NumberFormat('zh-CN', { notation: 'compact', maximumFractionDigits: 1 }).format(value)
const latency = (value: number | null) => value == null ? '—' : value >= 1000 ? `${(value / 1000).toFixed(1)} s` : `${whole(Math.round(value))} ms`
const updated = (value: number) => value ? formatShortDate(new Date(value).toISOString()) : '尚未更新'

function Metric({ label, value, detail, tone = 'neutral' }: { label: string; value: ReactNode; detail: ReactNode; tone?: string }) {
  return <div className="overview-metric"><dt>{label}</dt><dd className={`overview-metric-value overview-tone-${tone}`}>{value}</dd><dd className="overview-metric-detail">{detail}</dd></div>
}

function QueryFeedback({ error, hasData, onRetry }: { error: Error | null; hasData: boolean; onRetry: () => void }) {
  if (!error) return null
  return <div className="overview-query-error"><ErrorBox error={new Error(hasData ? `刷新失败，保留上次数据。${error.message}` : `加载失败。${error.message}`)} retry={onRetry} /></div>
}

export function OverviewPage() {
  const user = useUser()
  const keys = useKeys()
  const monitoring = useMonitoringSummary()
  const quality = useQualityMonitors()
  const onboarding = useOnboarding()
  const [searchParams, setSearchParams] = useSearchParams()
  const range = parseUsageRange(searchParams.get('range'))
  const failuresOnly = searchParams.get('status') === 'error'
  const rangeLabel = usageRanges.find(option => option.value === range)!.label
  const offset = -new Date().getTimezoneOffset()
  const filters = new URLSearchParams({ range, utc_offset_minutes: String(offset) }).toString()
  const eventFilters = `${filters}${failuresOnly ? '&status=error' : ''}`
  const summary = useQuery({ queryKey: ['usage-summary', user.account_id, filters], queryFn: ({ signal }) => api<UsageSummary>(`/usage/summary?${filters}`, { signal }) })
  const events = useQuery({ queryKey: ['usage-events', user.account_id, eventFilters, 1, 5], queryFn: ({ signal }) => api<UsageEventPage>(`/usage/events?${eventFilters}&page=1&limit=5`, { signal }) })
  const changeFilter = (name: string, value: string | null) => setSearchParams(previous => {
    const next = new URLSearchParams(previous)
    if (value) next.set(name, value); else next.delete(name)
    return next
  }, { replace: true })
  const fetching = [summary, events, monitoring, quality, keys].some(query => query.isFetching)
  const refresh = () => { void Promise.all([summary.refetch(), events.refetch(), monitoring.refetch(), quality.refetch(), keys.refetch()]) }
  const totals = summary.data?.totals
  const failed = totals ? Math.max(0, totals.requests - totals.successful) : 0
  const rate = totals?.requests ? totals.successful / totals.requests * 100 : null
  const percent = rate == null ? '—' : rate < 100 && rate.toFixed(1) === '100.0' ? '>99.9%' : `${rate.toFixed(1)}%`
  const pendingValue = summary.isPending ? '加载中…' : '—'
  const noUsage = totals ? '暂无请求' : summary.isPending ? '正在加载统计' : '统计加载失败'
  const hourly = (summary.data?.bucket_seconds ?? 3600) < 86400
  const tickDate = (value: string) => new Intl.DateTimeFormat('zh-CN', hourly ? { hour: '2-digit', minute: '2-digit', hourCycle: 'h23' } : { month: 'numeric', day: 'numeric' }).format(new Date(value))
  const timezone = new Intl.DateTimeFormat('zh-CN', { timeZoneName: 'shortOffset' }).formatToParts(new Date()).find(part => part.type === 'timeZoneName')?.value.replace('GMT', 'UTC')
  const channelRows = useMemo(() => (monitoring.data?.channels ?? []).map(channel => ({ channel, state: overviewChannelState(channel, quality.data) }))
    .sort((a, b) => a.state.priority - b.state.priority || a.channel.priority - b.channel.priority || a.channel.name.localeCompare(b.channel.name)), [monitoring.data, monitoring.dataUpdatedAt, quality.data, quality.dataUpdatedAt])
  const enabledChannels = channelRows.filter(row => row.channel.enabled)
  const attention = enabledChannels.filter(row => row.state.priority < 2)
  const unhealthy = attention.filter(row => row.state.tone === 'danger').length
  const pending = attention.length - unhealthy
  const healthy = enabledChannels.filter(({ channel }) => channel.availability_enabled && channel.availability_status === 'healthy' && channel.availability_checked_at).length
  const unmonitored = enabledChannels.filter(({ channel }) => !channel.availability_enabled).length
  const disabled = channelRows.length - enabledChannels.length
  const qualityEnabled = (quality.data ?? []).filter(monitor => monitor.channel_enabled).length
  const qualityDegraded = (quality.data ?? []).filter(monitor => monitor.channel_enabled && monitor.status === 'degraded' && monitor.valid_until && new Date(monitor.valid_until).getTime() > Date.now()).length
  const channelSummary = channelRows.length ? <>
    可用性正常 <strong>{healthy} / {enabledChannels.length}</strong>
    {unmonitored ? ` · 未监控 ${unmonitored}` : ''}{disabled ? ` · 已停用 ${disabled}` : ''}
  </> : '当前监控快照'
  const firstAttention = attention[0]
  const modelRows = summary.data?.models.slice(0, 3) ?? []

  return <div className="overview-page">
    <PageHeader title="运行概览" description="查看所选时段的请求表现，定位需要处理的渠道。" action={<div className="overview-header-actions"><div className="overview-range"><label htmlFor="overview-range">统计范围</label><Choice id="overview-range" name="range" value={range} onChange={value => changeFilter('range', value)} options={usageRanges} /></div><Button variant="outline" onClick={refresh} disabled={fetching}><RefreshCw aria-hidden="true" className={fetching ? 'animate-spin' : ''} />刷新数据</Button></div>} />

    <GettingStarted onboarding={onboarding} />

    {!(onboarding.visible && !channelRows.length && monitoring.isSuccess) && <section className="overview-signal" aria-label="运行状态" data-tone={monitoring.isError ? 'warning' : attention.length ? 'warning' : 'neutral'}>
      <div className="overview-signal-copy">
        {monitoring.isPending ? <><Network aria-hidden="true" /><div><h2>正在加载渠道状态…</h2><p>请求统计将独立加载。</p></div></> : monitoring.isError && !monitoring.data ? <><ShieldAlert aria-hidden="true" /><div><h2>渠道状态加载失败</h2><p>暂时无法判断渠道健康，请重试。</p></div></> : monitoring.isError ? <><ShieldAlert aria-hidden="true" /><div><h2>渠道状态刷新失败</h2><p>保留上次快照（{updated(monitoring.dataUpdatedAt)}），请重试。</p></div></> : !channelRows.length ? <><Network aria-hidden="true" /><div><h2>接入第一个上游渠道</h2><p>添加渠道、创建 API Key 后，即可开始调用。</p></div></> : firstAttention ? <><ShieldAlert aria-hidden="true" /><div><h2>{attention.length} 个渠道需要关注</h2><p>{unhealthy > 0 ? `${unhealthy} 个异常` : ''}{unhealthy > 0 && pending > 0 ? ' · ' : ''}{pending > 0 ? `${pending} 个待确认` : ''} · {firstAttention.channel.name}：{firstAttention.state.label}</p></div></> : <><CheckCircle2 aria-hidden="true" /><div><h2>{enabledChannels.length ? '暂未发现监控异常' : '暂无启用渠道'}</h2><p>已启用 {enabledChannels.length} 个渠道{unmonitored ? ` · ${unmonitored} 个未开启可用性监控` : ''} · 以最近检查结果为准</p></div></>}
      </div>
      {monitoring.isError && !monitoring.data ? <Button variant="outline" onClick={() => void monitoring.refetch()} disabled={monitoring.isFetching}>重试渠道状态</Button> : firstAttention ? <Link className="overview-detail-link" to={monitoringHref(firstAttention.channel.id, firstAttention.state.kind)}>查看异常详情<ArrowRight aria-hidden="true" /></Link> : <Link className="overview-detail-link" to={channelRows.length ? '/monitoring' : '/channels'}>{channelRows.length ? '查看渠道监控' : '添加渠道'}<ArrowRight aria-hidden="true" /></Link>}
    </section>}

    <QueryFeedback error={quality.error} hasData={!!quality.data} onRetry={() => void quality.refetch()} />

    <section aria-label="请求统计" aria-busy={summary.isFetching}>
      <QueryFeedback error={summary.error} hasData={!!summary.data} onRetry={() => void summary.refetch()} />
      <dl className="overview-metrics">
        <Metric label="请求量" value={totals ? whole(totals.requests) : pendingValue} detail={totals ? rangeLabel : noUsage} />
        <Metric label="成功率" value={totals ? percent : pendingValue} detail={totals?.requests ? `${whole(totals.successful)} / ${whole(totals.requests)} 次成功` : noUsage} tone={totals ? successTone(totals.successful, totals.requests) : 'neutral'} />
        <Metric label="失败请求" value={totals ? whole(failed) : pendingValue} detail={failed ? <Link className="overview-detail-link" to={usageHref(range, { status: 'error' })}>查看失败调用<ArrowRight aria-hidden="true" /></Link> : totals?.requests ? '当前范围内无失败' : noUsage} tone={failed ? 'danger' : 'neutral'} />
        <Metric label="平均首字延迟" value={totals ? latency(totals.avg_ttfb_ms) : pendingValue} detail={totals?.requests ? `总耗时均值 ${latency(totals.avg_duration_ms)}` : noUsage} />
      </dl>
      <p className="overview-data-note" role="status">{summary.isFetching ? '正在更新用量统计…' : `用量更新：${updated(summary.dataUpdatedAt)}`} · {timezone} · {range === '24h' ? '按小时分桶，本小时尚未结束' : '按自然日分桶，今日尚未结束'}</p>
    </section>

    <div className="overview-main-grid">
      <Panel className="overview-trend-card">
        <div className="overview-card-heading"><div><h2>请求趋势</h2><p>{rangeLabel} · 按{hourly ? '小时' : '天'}聚合，包含零请求时段</p></div><Link className="overview-detail-link" to={usageHref(range)}>使用分析<ArrowRight aria-hidden="true" /></Link></div>
        {summary.isPending ? <div className="overview-loading"><Loading /></div> : !summary.data ? <div className="overview-section-message">趋势暂不可用，请重试上方的用量统计。</div> : !totals?.requests ? <div className="overview-card-fill"><Empty title="当前时段暂无请求" description="可切换时间范围，或在客户端发起一次模型调用。" action={<Button asChild variant="outline"><Link to="/keys">查看 API Key</Link></Button>} /></div> : <>
          <div className="overview-chart" aria-label={`${rangeLabel}请求趋势，共 ${whole(totals.requests)} 次请求`}>
            <ResponsiveContainer width="100%" height="100%"><BarChart data={summary.data.trend} accessibilityLayer margin={{ top: 16, right: 12, left: 0, bottom: 0 }}>
              <CartesianGrid stroke="var(--border)" vertical={false} />
              <XAxis dataKey="start" tickFormatter={tickDate} tick={{ fill: 'var(--muted-foreground)', fontSize: 12 }} tickLine={false} axisLine={false} interval="preserveStartEnd" minTickGap={24} />
              <YAxis width={46} allowDecimals={false} tickFormatter={value => usageAxisTick(Number(value))} tick={{ fill: 'var(--muted-foreground)', fontSize: 12 }} tickLine={false} axisLine={false} />
              <Tooltip cursor={{ fill: 'var(--muted)' }} contentStyle={{ background: 'var(--popover)', border: '1px solid var(--border)', borderRadius: 8, color: 'var(--foreground)', fontSize: 12 }} labelFormatter={label => formatDate(String(label))} formatter={value => [whole(Number(value)), '请求数']} />
              <Bar dataKey="requests" fill="var(--primary)" maxBarSize={28} radius={[3, 3, 0, 0]} isAnimationActive={false} />
            </BarChart></ResponsiveContainer>
          </div>
        </>}
      </Panel>

      <Panel className="overview-health-card">
        <div className="overview-card-heading"><div><h2>渠道状态</h2><p>{channelSummary}</p></div><Link to="/monitoring" className="overview-detail-link">查看全部</Link></div>
        <QueryFeedback error={monitoring.error} hasData={!!monitoring.data} onRetry={() => void monitoring.refetch()} />
        {monitoring.isPending ? <div className="overview-loading"><Loading /></div> : !monitoring.data ? null : !channelRows.length ? <div className="overview-card-fill"><Empty title="还没有渠道" description="接入模型供应商后，这里会展示监控状态。" /></div> : <>
          <div className="overview-health-list">{channelRows.slice(0, 3).map(({ channel, state }) => <Link key={channel.id} to={monitoringHref(channel.id, state.kind)} className="overview-health-row" aria-label={`${channel.name}，${state.label}，查看监控`}><div className="min-w-0"><p className="overview-channel-name">{channel.name}</p><p className="overview-channel-detail">{state.priority < 2 ? state.detail : null}</p><p className="overview-channel-time">{protocolLabels[channel.protocol]} · {state.checkedAt ? `${formatShortDate(state.checkedAt)} 检查` : '尚无检查记录'}</p></div><Badge tone={state.tone}>{state.label}</Badge></Link>)}</div>
          <div className="overview-health-footer"><span>质量检测：{quality.isPending ? '加载中…' : quality.isError && !quality.data ? '加载失败' : qualityEnabled ? `${qualityEnabled} 个模型监控 · ${qualityDegraded} 个异常` : '未添加'}</span><span>快照更新：{updated(monitoring.dataUpdatedAt)}</span></div>
        </>}
      </Panel>
    </div>

    <Panel className="overview-recent-card">
      <div className="overview-card-heading"><div><h2>最近调用</h2><p>{rangeLabel}内最新 5 条{failuresOnly ? '失败' : ''}请求</p></div><div className="overview-recent-actions"><div className="overview-status-filter" role="group" aria-label="最近调用筛选"><Button variant={!failuresOnly ? 'secondary' : 'ghost'} aria-pressed={!failuresOnly} onClick={() => changeFilter('status', null)}>全部</Button><Button variant={failuresOnly ? 'secondary' : 'ghost'} aria-pressed={failuresOnly} onClick={() => changeFilter('status', 'error')}>仅失败</Button></div><Link className="overview-detail-link" to={usageHref(range, { status: failuresOnly ? 'error' : undefined })}>查看全部<ArrowRight aria-hidden="true" /></Link></div></div>
      <QueryFeedback error={events.error} hasData={!!events.data} onRetry={() => void events.refetch()} />
      {events.isPending ? <div className="overview-loading"><Loading /></div> : !events.data ? null : !events.data.items.length ? <Empty title={failuresOnly ? '当前时段没有失败请求' : '当前时段暂无调用'} description={failuresOnly ? '可切换为“全部”查看其他请求。' : '切换时间范围，或在客户端发起一次模型调用。'} /> : <div className="overview-table-scroll"><table className="overview-events-table"><thead><tr><th scope="col">时间</th><th scope="col">模型 / 渠道</th><th scope="col">状态</th><th scope="col" className="text-right">Token</th><th scope="col" className="text-right">首字 / 总耗时</th></tr></thead><tbody>{events.data.items.map(row => {
        const success = row.status >= 200 && row.status < 300
        return <tr key={row.id}><td className="tabular-nums"><time dateTime={row.created_at} title={formatDate(row.created_at)}>{formatShortDate(row.created_at)}</time></td><td><Link to={usageHref(range, { channel: row.channel_id, model: row.model, status: failuresOnly ? 'error' : undefined })} className="overview-request-link">{row.model}</Link><p className="overview-channel-detail">{row.channel_name ?? (row.channel_id ? '已删除渠道' : '未记录渠道')} · {protocolLabels[row.protocol]}</p></td><td><Badge tone={success ? 'success' : row.status >= 400 && row.status < 500 ? 'warning' : 'danger'}>{row.status} {success ? '成功' : '失败'}</Badge></td><td className="text-right tabular-nums"><span className="overview-mobile-label">Token </span>{row.input_tokens == null && row.output_tokens == null ? '未上报' : compact((row.input_tokens ?? 0) + (row.output_tokens ?? 0))}</td><td className="text-right tabular-nums"><span className="overview-mobile-label">首字 </span>{latency(row.ttfb_ms)}<p className="overview-channel-detail">总耗时 {latency(row.duration_ms)}</p></td></tr>
      })}</tbody></table></div>}
      {events.data && <p className="overview-data-note overview-events-note" role="status">{events.isFetching ? '正在更新调用记录…' : `${failuresOnly ? '失败' : '全部'}请求共 ${whole(events.data.total)} 条 · 更新于 ${updated(events.dataUpdatedAt)}`}</p>}
    </Panel>

    <div className="overview-secondary">
      <div className="overview-resource-links"><Link to="/keys"><KeyRound aria-hidden="true" />已启用 API Key {keys.data ? keys.data.filter(key => key.enabled && !key.revoked_at).length : '—'}</Link><Link to="/channels"><Network aria-hidden="true" />已启用渠道 {monitoring.data ? enabledChannels.length : '—'}</Link><Link to={usageHref(range)}>查看模型分布<ArrowRight aria-hidden="true" /></Link></div>
      <QueryFeedback error={keys.error} hasData={!!keys.data} onRetry={() => void keys.refetch()} />
      {modelRows.length > 0 && <p className="overview-model-summary">常用模型：{modelRows.map((model, index) => <span key={model.model}>{index > 0 ? ' · ' : ''}<Link to={usageHref(range, { model: model.model })}>{model.model}</Link> {totals?.requests ? `${(model.requests / totals.requests * 100).toFixed(1)}%` : '—'}</span>)}</p>}
    </div>
  </div>
}
