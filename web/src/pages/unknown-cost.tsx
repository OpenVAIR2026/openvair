import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { ChevronLeft, ChevronRight } from 'lucide-react'
import { api, formatShortDate } from '../lib/api'
import { Choice, ErrorBox, Loading, Panel } from '../components/common'
import { Button } from '../components/ui/button'

interface Option { value: string; name?: string | null; account_name?: string | null; requests: number }
interface UnknownCostReport {
  page: number
  page_size: number
  totals: { successful: number; missing_usage: number; unpriced: number; priced_cost_usd: number }
  reasons: Option[]
  channels: Option[]
  keys: Option[]
  rows: { id: string; created_at: string; account_name: string | null; key_name: string | null; channel_name: string | null; model: string; completion: string; duration_ms: number; first_token_ms: number | null; input_tokens: number | null; output_tokens: number | null }[]
}

const ALL = 'all'
const RANGES = [{ value: '1', label: '今天' }, { value: '7', label: '近 7 天' }, { value: '30', label: '近 30 天' }, { value: '90', label: '近 90 天' }]
const REASONS: Record<string, string> = {
  client_disconnected: '客户端断开',
  success: '正常结束但无用量',
  stream_error: '上游断流',
  unfinished: '未结束',
  unrecorded: '未记录',
}

const whole = (value: number) => value.toLocaleString('zh-CN')
const share = (part: number, total: number) => total ? `${(part / total * 100).toFixed(2)}%` : '—'
const seconds = (ms: number | null) => ms == null ? '—' : `${(ms / 1000).toFixed(1)}s`
const tokens = (value: number | null) => value == null ? '—' : whole(value)
const reason = (completion: string) => REASONS[completion] ?? completion
const options = (all: string, items: Option[] | undefined, label: (item: Option) => string) =>
  [{ value: ALL, label: all }, ...(items ?? []).map(item => ({ value: item.value, label: `${label(item)}（${whole(item.requests)}）` }))]
const named = (item: Option) => item.account_name ? `${item.name ?? '—'} · ${item.account_name}` : item.name ?? '—'

/** Successful requests whose upstream reported no usage, platform-wide; shown only to super admins. */
export function UnknownCostPanel({ onPrice }: { onPrice: () => void }) {
  const [filters, setFilters] = useState({ days: '7', completion: ALL, channel_id: ALL, key_id: ALL })
  const [page, setPage] = useState(0)
  const params = new URLSearchParams({ days: filters.days, page: String(page) })
  for (const name of ['completion', 'channel_id', 'key_id'] as const) if (filters[name] !== ALL) params.set(name, filters[name])
  const report = useQuery({
    queryKey: ['platform-unknown-cost', params.toString()],
    queryFn: ({ signal }) => api<UnknownCostReport>(`/platform/unknown-cost?${params}`, { signal }),
    placeholderData: previous => previous,
  })
  const data = report.data
  const filter = (name: keyof typeof filters) => (value: string) => { setFilters(current => ({ ...current, [name]: value })); setPage(0) }
  const pages = data ? Math.max(1, Math.ceil(data.totals.missing_usage / data.page_size)) : 1
  return <div className="grid gap-4">
    <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
      <Choice aria-label="时间范围" value={filters.days} onChange={filter('days')} options={RANGES} />
      <Choice aria-label="原因" value={filters.completion} onChange={filter('completion')} options={options('全部原因', data?.reasons, item => reason(item.value))} />
      <Choice aria-label="渠道" value={filters.channel_id} onChange={filter('channel_id')} options={options('全部渠道', data?.channels, named)} />
      <Choice aria-label="API Key" value={filters.key_id} onChange={filter('key_id')} options={options('全部 Key', data?.keys, named)} />
    </div>
    <ErrorBox error={report.error} retry={() => report.refetch()} />
    {report.isPending ? <Loading /> : data ? <>
      <p className="text-sm text-muted-foreground">
        缺少用量 <span className="font-medium text-foreground">{whole(data.totals.missing_usage)}</span> 条，占成功请求 {share(data.totals.missing_usage, data.totals.successful)}
        {' · '}未定价 {whole(data.totals.unpriced)} 条 <Button variant="link" className="h-auto p-0 text-sm" onClick={onPrice}>去定价</Button>
        {' · '}已计价 ${data.totals.priced_cost_usd.toFixed(2)}
      </p>
      <Panel className="overflow-hidden">
        {data.rows.length ? <div className="overflow-x-auto"><table className="w-full text-sm">
          <thead className="text-xs text-muted-foreground"><tr>{['时间', 'Key', '渠道 · 模型', '原因', '首字 / 耗时', '输入 / 输出 Token'].map((label, index) => <th key={label} className={`whitespace-nowrap px-5 py-3 font-medium ${index < 4 ? 'text-left' : 'text-right'}`}>{label}</th>)}</tr></thead>
          <tbody>{data.rows.map(row => <tr key={row.id} className="border-t border-border">
            <td className="whitespace-nowrap px-5 py-2 tabular-nums">{formatShortDate(row.created_at)}</td>
            <td className="px-5 py-2"><div className="grid gap-0.5"><span className="font-medium">{row.key_name ?? '—'}</span>{row.account_name && <span className="text-xs text-muted-foreground">{row.account_name}</span>}</div></td>
            <td className="px-5 py-2"><div className="grid gap-0.5"><span>{row.channel_name ?? '—'}</span><span className="break-all font-mono text-xs text-muted-foreground">{row.model}</span></div></td>
            <td className="whitespace-nowrap px-5 py-2">{reason(row.completion)}</td>
            <td className="whitespace-nowrap px-5 py-2 text-right tabular-nums">{seconds(row.first_token_ms)} / {seconds(row.duration_ms)}</td>
            <td className="whitespace-nowrap px-5 py-2 text-right tabular-nums">{tokens(row.input_tokens)} / {tokens(row.output_tokens)}</td>
          </tr>)}</tbody>
        </table></div> : <p className="px-5 py-6 text-sm text-muted-foreground">没有缺少用量的请求</p>}
        <div className="flex items-center justify-between border-t border-border px-5 py-3 text-sm text-muted-foreground">
          <span>第 {page + 1} / {pages} 页</span>
          <div className="flex gap-2">
            <Button size="icon" variant="outline" aria-label="上一页" disabled={page === 0} onClick={() => setPage(page - 1)}><ChevronLeft aria-hidden="true" /></Button>
            <Button size="icon" variant="outline" aria-label="下一页" disabled={page + 1 >= pages} onClick={() => setPage(page + 1)}><ChevronRight aria-hidden="true" /></Button>
          </div>
        </div>
      </Panel>
    </> : null}
  </div>
}
