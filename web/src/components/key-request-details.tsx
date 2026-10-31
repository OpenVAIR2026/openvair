import { X, RefreshCw } from 'lucide-react'
import { useQuery } from '@tanstack/react-query'
import { formatDate, type Protocol, type RequestOutcome } from '../lib/api'
import { Badge, ErrorBox, Loading } from './common'
import { Button } from './ui/button'
import { Sheet, SheetClose, SheetContent, SheetDescription, SheetHeader, SheetTitle } from './ui/sheet'

export interface KeyRequestDetail {
  id: string
  model: string
  protocol: Protocol
  status: number
  created_at: string
  stream: boolean | null
  ttfb_ms: number | null
  first_token_ms: number | null
  duration_ms: number
  input_tokens: number | null
  output_tokens: number | null
  cached_tokens: number | null
  cost_usd: number | null
  timing: { queue_ms: number | null; upstream_headers_ms: number | null; headers_to_first_token_ms: number | null; completion: RequestOutcome | null } | null
}

const ms = (value: number | null) => value == null ? '未采集' : value >= 1000 ? `${(value / 1000).toFixed(3)} s` : `${value} ms`
const outcomes: Record<RequestOutcome, string> = { pending: '响应中', success: '已完成', http_error: '请求失败', timeout: '请求超时', first_token_timeout: '首字超时，已切换', connection_error: '连接失败', stream_error: '响应中断', client_disconnected: '客户端断开', unavailable: '服务不可用' }

export function KeyRequestDetails({ requestId, cacheId, load, onClose, returnFocus }: { requestId: string | null; cacheId: string; load: (id: string, signal: AbortSignal) => Promise<KeyRequestDetail>; onClose: () => void; returnFocus: () => void }) {
  const query = useQuery({ queryKey: ['key-holder', 'request-detail', cacheId, requestId], enabled: !!requestId, retry: false,
    queryFn: ({ signal }) => load(requestId!, signal),
    refetchInterval: query => query.state.data?.timing?.completion === null ? 3000 : false,
  })
  const detail = query.isError ? undefined : query.data
  const completion = detail?.timing?.completion
  const first = detail?.stream ? detail.first_token_ms : null
  const rows = detail ? [
    ['请求 → 响应头', detail.ttfb_ms],
    ['并发排队', detail.timing?.queue_ms ?? null],
    ['上游响应等待', detail.timing?.upstream_headers_ms ?? null],
    ...(detail.stream ? [['响应头 → 首个输出', detail.timing?.headers_to_first_token_ms ?? null], ['首个输出 → 结束', first != null && completion != null ? Math.max(0, detail.duration_ms - first) : null]] : []),
  ] as [string, number | null][] : []
  return <Sheet open={!!requestId} onOpenChange={open => { if (!open) onClose() }}><SheetContent side="right" showCloseButton={false} className="w-full gap-0 p-0 sm:max-w-[560px]" onCloseAutoFocus={event => { event.preventDefault(); returnFocus() }}>
    <SheetHeader className="shrink-0 gap-2 border-b border-border px-6 py-5 pr-16"><SheetTitle>请求详情</SheetTitle><SheetDescription>查看这次请求的耗时、用量和费用</SheetDescription><SheetClose asChild><Button variant="ghost" size="icon" className="absolute right-4 top-4" aria-label="关闭请求详情"><X aria-hidden="true" /></Button></SheetClose></SheetHeader>
    <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-5 py-5 sm:px-6">
      <ErrorBox error={query.error} retry={() => { void query.refetch() }} />
      {query.isPending ? <Loading /> : detail && <div className="space-y-7">
        <section className="space-y-4"><div className="flex flex-wrap items-center gap-2"><h3 className="break-all text-lg font-semibold">{detail.model}</h3><Badge>HTTP {detail.status}</Badge>{detail.timing && <Badge tone={completion === 'success' ? 'success' : 'warning'}>{completion ? outcomes[completion] : '尚未结束'}</Badge>}</div>
          <div className="grid grid-cols-2 gap-4 border-y border-border py-4"><div><p className="text-xs text-muted-foreground">{detail.stream ? '首个输出' : '收到响应头'}</p><p className="mt-2 text-2xl font-semibold tabular-nums">{ms(detail.stream ? first : detail.ttfb_ms)}</p></div><div><p className="text-xs text-muted-foreground">{detail.timing && !completion ? '已记录耗时' : '总耗时'}</p><p className="mt-2 text-2xl font-semibold tabular-nums">{ms(detail.duration_ms)}</p></div></div>
          {completion && completion !== 'success' && detail.status < 300 && <p className="text-sm text-warning">上游已返回成功状态，但本次响应未完整结束。</p>}
          <p className="text-sm text-muted-foreground">{formatDate(detail.created_at)} · {({ openai: 'OpenAI', anthropic: 'Anthropic', gemini: 'Gemini' })[detail.protocol]} · {detail.stream == null ? '未记录流式状态' : detail.stream ? '流式' : '非流式'}</p>
        </section>
        <section><h3 className="font-semibold">耗时分解</h3>{!detail.timing && <p className="mt-3 text-sm text-muted-foreground">未采集分段计时，历史请求无法还原。</p>}<dl className="mt-3 divide-y divide-border">{rows.map(([label, value]) => <div key={label} className="flex items-center justify-between gap-3 py-3 text-sm"><dt className="text-muted-foreground">{label}</dt><dd className="shrink-0 font-medium tabular-nums">{ms(value)}</dd></div>)}</dl><p className="mt-3 text-xs leading-5 text-muted-foreground">首个输出包括文本、推理或工具调用。响应头不等于首个输出；累计时间与分段时间不应相加。上游响应等待从最终上游请求发出开始计量。</p></section>
        <section><h3 className="font-semibold">用量与费用</h3><dl className="mt-3 grid grid-cols-2 gap-4 text-sm">{[['输入 Token', detail.input_tokens?.toLocaleString('zh-CN') ?? '未上报'], ['缓存 Token', detail.cached_tokens?.toLocaleString('zh-CN') ?? '未上报'], ['输出 Token', detail.output_tokens?.toLocaleString('zh-CN') ?? '未上报'], ['费用', detail.cost_usd == null ? '—' : `$${detail.cost_usd.toFixed(6)}`]].map(([label, value]) => <div key={label}><dt className="text-xs text-muted-foreground">{label}</dt><dd className="mt-1 font-medium tabular-nums">{value}</dd></div>)}</dl></section>
        <div className="border-t border-border pt-4"><p className="text-xs text-muted-foreground">请求记录 ID</p><p className="mt-1 select-all break-all font-mono text-xs text-muted-foreground">{detail.id}</p></div>
      </div>}
    </div>
    <div className="flex shrink-0 justify-end border-t border-border px-6 py-3"><Button variant="ghost" size="sm" disabled={query.isFetching} onClick={() => { void query.refetch() }}><RefreshCw aria-hidden="true" />{query.isFetching ? '更新中…' : '刷新详情'}</Button></div>
  </SheetContent></Sheet>
}
