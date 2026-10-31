import { useState } from 'react'
import { formatShortDate, type QuotaEvent } from '../lib/api'
import { usageCost } from '../lib/usage-metrics'
import { Button } from './ui/button'
import { ErrorBox, Loading } from './common'

// Matches the API Key page: cents for ordinary amounts, more digits below one cent.
const usd = (value: number) => value === 0 || value >= 0.01 ? `$${value.toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 })}` : usageCost(value)
const limit = (value: number | null) => value == null ? '不限' : usd(value)

function describe(event: QuotaEvent): [string, string] {
  switch (event.kind) {
    case 'increase': return [`增加 ${usd(event.amount_usd ?? 0)}`, `上限 ${limit(event.limit_before_usd)} → ${limit(event.limit_after_usd)}`]
    case 'reset': return ['重置累计用量', `重置前已用 ${usd(event.used_before_usd ?? 0)}，上限 ${limit(event.limit_after_usd)}`]
    case 'opening': return ['开始记录', `当时上限 ${limit(event.limit_after_usd)}`]
    default:
      if (event.limit_before_usd == null) return ['设置上限', `上限 ${limit(event.limit_after_usd)}`]
      if (event.limit_after_usd == null) return ['取消上限', `原上限 ${limit(event.limit_before_usd)}`]
      return ['调整上限', `上限 ${limit(event.limit_before_usd)} → ${limit(event.limit_after_usd)}`]
  }
}

/**
 * Changes to a key's total cost limit, newest first. `initial` caps how many
 * show before the holder asks for the rest.
 */
export function QuotaHistory({ events, pending, error, retry, initial = Infinity, empty = '暂无额度记录' }: { events: QuotaEvent[] | undefined; pending: boolean; error: Error | null; retry?: () => void; initial?: number; empty?: string }) {
  const [all, setAll] = useState(false)
  if (pending) return <Loading />
  if (error) return <ErrorBox error={error} retry={retry} />
  if (!events?.length) return <p className="text-sm text-muted-foreground">{empty}</p>
  const shown = all ? events : events.slice(0, initial)
  return <div className="grid gap-2">
    <ol className="grid divide-y divide-border rounded-lg border border-border">
      {shown.map(event => {
        const [title, detail] = describe(event)
        return <li key={event.id} className="grid gap-0.5 px-3 py-2.5 text-sm">
          <div className="flex flex-wrap items-baseline justify-between gap-x-3"><span className={`font-medium${event.kind === 'increase' ? ' text-success' : ''}`}>{title}</span><time className="text-xs text-muted-foreground tabular-nums" dateTime={event.created_at}>{formatShortDate(event.created_at)}</time></div>
          <span className="text-xs text-muted-foreground tabular-nums">{detail}</span>
          {event.note && <span className="break-words text-xs">备注：{event.note}</span>}
        </li>
      })}
    </ol>
    {shown.length < events.length && <div><Button type="button" variant="ghost" size="sm" onClick={() => setAll(true)}>显示全部 {events.length} 条</Button></div>}
  </div>
}
