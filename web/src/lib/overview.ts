import { HEALTH_ERROR_LABELS, type MonitoringChannel, type QualityMonitor, type UsageRange } from './api'

export const usageRanges: { value: UsageRange; label: string }[] = [
  { value: '24h', label: '近 24 小时' },
  { value: '7d', label: '近 7 天' },
  { value: '30d', label: '近 30 天' },
]

export function parseUsageRange(value: string | null): UsageRange {
  return value === '7d' || value === '30d' ? value : '24h'
}

export function usageHref(range: UsageRange, filters: { channel?: string | null; model?: string; status?: string } = {}) {
  const params = new URLSearchParams({ range })
  for (const [name, value] of Object.entries(filters)) if (value && value !== 'all') params.set(name, value)
  return `/usage?${params}`
}

type ChannelState = {
  label: string
  tone: 'neutral' | 'success' | 'warning' | 'danger'
  priority: number
  detail: string
  checkedAt: string | null
  kind: 'availability' | 'quality'
}

type QualitySnapshot = Pick<QualityMonitor, 'channel_id' | 'model' | 'status' | 'checked_at' | 'valid_until' | 'last_error' | 'schedule_enabled'>

/** Summarize observed health, never infer route eligibility from a single probe. */
export function overviewChannelState(channel: MonitoringChannel, monitors: QualitySnapshot[] = [], now = Date.now()): ChannelState {
  const availability = { checkedAt: channel.availability_checked_at, kind: 'availability' as const }
  if (!channel.enabled) return { ...availability, label: '已停用', tone: 'neutral', priority: 3, detail: '不参与当前渠道统计' }
  if (channel.availability_enabled && channel.availability_status === 'unhealthy') {
    const label = channel.availability_error_kind ? HEALTH_ERROR_LABELS[channel.availability_error_kind] : '可用性异常'
    return { ...availability, label, tone: 'danger', priority: 0, detail: channel.availability_error || '最近连通检查未通过' }
  }
  const channelMonitors = monitors.filter(monitor => monitor.channel_id === channel.id)
  // The latest result stands until the next run; it does not expire on screen.
  const degraded = channelMonitors.find(monitor => monitor.status === 'degraded')
  if (degraded) {
    return { checkedAt: degraded.checked_at, kind: 'quality', label: '质量异常', tone: 'danger', priority: 0, detail: `${degraded.model}：${degraded.last_error || '质量检查已达到异常确认阈值'}` }
  }
  if (channel.availability_enabled && channel.availability_status === 'model_unavailable') {
    return { ...availability, label: '模型无效', tone: 'warning', priority: 1, detail: channel.availability_error || '测试模型在上游不可用，请更换测试模型' }
  }
  if (channel.availability_enabled && channel.availability_status === 'rate_limited') {
    return { ...availability, label: '上游限流', tone: 'warning', priority: 1, detail: channel.availability_error || '最近检查被上游限流，仍参与路由' }
  }
  if (channel.availability_enabled && (channel.availability_status === 'unknown' || !channel.availability_checked_at)) {
    return { ...availability, label: '待检查', tone: 'warning', priority: 1, detail: '尚无明确的可用性检查结果' }
  }
  // Manual-only monitors are expected to have no current verdict between runs.
  const scheduled = channelMonitors.filter(monitor => monitor.schedule_enabled)
  const pending = scheduled.find(monitor => monitor.status === 'unknown' || !monitor.checked_at || !monitor.valid_until)
  if (pending) {
    return { checkedAt: pending.checked_at, kind: 'quality', label: '服务体检待确认', tone: 'warning', priority: 1, detail: `${pending.model}：${pending.last_error || '尚无有效的服务体检结果'}` }
  }
  if (!channel.availability_enabled) return { ...availability, label: '未监控可用性', tone: 'neutral', priority: 2, detail: '可在渠道监控中开启' }
  return { ...availability, label: '正常', tone: 'success', priority: 2, detail: '最近可用性检查通过' }
}

export function monitoringHref(channelId: string, kind: ChannelState['kind']) {
  return `/monitoring?${new URLSearchParams({ channel: channelId, tab: kind })}`
}
