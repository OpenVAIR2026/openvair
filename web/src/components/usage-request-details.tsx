import { useQuery } from '@tanstack/react-query'
import { RefreshCw, X } from 'lucide-react'
import { api, formatDate, type RequestOutcome, type UsageRequestDetail } from '../lib/api'
import { Badge, ErrorBox, HelpTip, Loading } from './common'
import { Button } from './ui/button'
import { Sheet, SheetClose, SheetContent, SheetDescription, SheetHeader, SheetTitle } from './ui/sheet'

const outcomes: Record<RequestOutcome, string> = {
  pending: '响应中', success: '已完成', http_error: '上游错误', timeout: '连接或响应超时',
  first_token_timeout: '首字超时，已切换渠道', connection_error: '连接或发送失败', stream_error: '响应中断', client_disconnected: '客户端断开', unavailable: '渠道不可用',
}
const ms = (value: number | null | undefined) => value == null ? '未采集' : value >= 1000 ? `${(value / 1000).toFixed(3)} s` : `${value} ms`
const tokens = (value: number | null) => value == null ? '未上报' : value.toLocaleString('zh-CN')

function Timing({ label, value, help }: { label: string; value: number | null; help?: string }) {
  return <div className="flex items-center justify-between gap-4 py-2 text-sm"><dt className="flex items-center gap-1 text-muted-foreground">{label}{help && <HelpTip label={label}>{help}</HelpTip>}</dt><dd className="shrink-0 font-medium tabular-nums">{ms(value)}</dd></div>
}

function Details({ detail }: { detail: UsageRequestDetail }) {
  const diagnostics = detail.request_diagnostics
  const attempts = diagnostics?.attempts ?? []
  const last = attempts.at(-1)
  const complete = diagnostics?.completion != null
  const first = detail.stream ? detail.first_token_ms : null
  const output = first != null && complete ? Math.max(0, detail.duration_ms - first) : null
  const prior = attempts.slice(0, -1).reduce((sum, attempt) => sum + attempt.duration_ms, 0)
  const interrupted = diagnostics?.completion && !['success', 'http_error'].includes(diagnostics.completion)
  const stages = diagnostics && complete ? [
    { label: '并发排队', value: diagnostics.queue_ms, color: 'bg-warning' },
    { label: '前序尝试', value: prior, color: 'bg-destructive' },
    { label: '最后尝试', value: last?.duration_ms ?? 0, color: 'bg-primary' },
  ] : []
  const measured = stages.reduce((sum, stage) => sum + (stage.value ?? 0), 0)
  if (stages.length) stages.push({ label: '其他处理', value: Math.max(0, detail.duration_ms - measured), color: 'bg-muted-foreground' })

  return <div className="space-y-7">
    <section aria-label="请求概况" className="space-y-4">
      <div className="flex flex-wrap items-center gap-2"><h3 className="min-w-0 break-all text-lg font-semibold">{detail.model}</h3><Badge tone={detail.status >= 400 || interrupted ? 'warning' : 'success'}>HTTP {detail.status}</Badge>{diagnostics && <Badge tone={interrupted ? 'warning' : 'neutral'}>{diagnostics.completion ? outcomes[diagnostics.completion] : '尚未结束'}</Badge>}</div>
      <div className="grid grid-cols-2 gap-4 border-y border-border py-4">
        <div><p className="text-xs text-muted-foreground">{detail.stream ? '首个输出' : '收到响应头'}</p><p className="mt-2 text-2xl font-semibold tabular-nums">{ms(detail.stream ? first : detail.ttfb_ms)}</p></div>
        <div><p className="text-xs text-muted-foreground">{diagnostics && !complete ? '已记录耗时' : '总耗时'}</p><p className="mt-2 text-2xl font-semibold tabular-nums">{ms(detail.duration_ms)}</p></div>
      </div>
      {interrupted && <p className="text-sm text-warning">{detail.status < 300 ? '上游已返回成功状态，但本次响应未完整结束。' : '本次请求未完成，可在渠道尝试中查看失败阶段。'}</p>}
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-5 gap-y-2 text-sm">
        <dt className="text-muted-foreground">渠道</dt><dd className="break-all">{detail.channel_name ?? '已删除或未记录'}</dd>
        <dt className="text-muted-foreground">API Key</dt><dd className="break-all">{detail.key_name ?? '已删除或未记录'}{detail.key_revoked && '（已撤销）'}</dd>
        <dt className="text-muted-foreground">上游模型</dt><dd className="break-all">{detail.upstream_model ?? '未记录'}</dd>
        <dt className="text-muted-foreground">请求方式</dt><dd>{({ openai: 'OpenAI', anthropic: 'Anthropic', gemini: 'Gemini' })[detail.protocol]} · {detail.stream == null ? '未记录流式状态' : detail.stream ? '流式' : '非流式'}</dd>
        <dt className="text-muted-foreground">记录时间</dt><dd>{formatDate(detail.created_at)}</dd>
      </dl>
    </section>

    <section aria-labelledby="request-timing-title">
      <h3 id="request-timing-title" className="font-semibold">耗时分解</h3>
      <dl className="mt-2"><Timing label="请求 → 响应头" value={detail.ttfb_ms} help="从网关接收请求到最终成功上游返回响应头，包含并发排队、网关处理和前序尝试。" /></dl>
      {!diagnostics ? <p className="mt-3 rounded-lg bg-muted/40 p-4 text-sm text-muted-foreground">未采集分段计时。历史请求无法还原排队和渠道尝试耗时。</p> : <>
        {!!stages.length && <div className="my-4" aria-label="请求耗时分布"><div className="flex h-2 overflow-hidden rounded-full bg-muted">{stages.filter(stage => stage.value != null && stage.value > 0).map(stage => <span key={stage.label} className={stage.color} style={{ width: `${(stage.value ?? 0) / Math.max(detail.duration_ms, measured, 1) * 100}%` }} title={`${stage.label} ${ms(stage.value)}`} />)}</div><div className="mt-2 flex flex-wrap gap-x-4 gap-y-2 text-xs text-muted-foreground">{stages.map(stage => <span key={stage.label} className="inline-flex items-center gap-1.5"><span className={`size-1.5 rounded-full ${stage.color}`} />{stage.label} {ms(stage.value)}</span>)}</div></div>}
        {!complete && <p className="mt-3 text-sm text-muted-foreground">请求尚未结束，仅显示已采集的时间。</p>}
        <dl className="mt-2 divide-y divide-border">
          <Timing label="并发排队" value={diagnostics.queue_ms} help="等待此 API Key 的并发额度。跨帐号转发的来源记录不包含调用方的排队计时。" />
          <Timing label="前序渠道尝试" value={prior} help="最后一次尝试之前，所有已结束尝试的耗时之和。" />
          <Timing label="最后渠道 → 响应头" value={last?.headers_ms ?? null} help="从最后一次上游请求发出到收到响应头，包含网络传输和上游等待。" />
          {detail.stream && <Timing label="响应头 → 首个输出" value={last?.first_token_ms != null && last.headers_ms != null ? Math.max(0, last.first_token_ms - last.headers_ms) : null} />}
          {detail.stream && <Timing label="首个输出 → 结束" value={output} />}
        </dl>
      </>}
      <p className="mt-3 text-xs leading-5 text-muted-foreground">响应头不等于首个输出。首个输出包括文本、推理或工具调用；耗时从网关接收请求开始计量。</p>
    </section>

    <section aria-labelledby="request-attempts-title">
      <div className="flex items-center justify-between"><h3 id="request-attempts-title" className="font-semibold">渠道尝试</h3>{diagnostics && <span className="text-xs text-muted-foreground">{attempts.length} 次尝试</span>}</div>
      {!diagnostics ? <p className="mt-3 text-sm text-muted-foreground">未采集渠道尝试记录。</p> : !attempts.length ? <p className="mt-3 text-sm text-muted-foreground">没有已记录的渠道尝试。</p> : <ol className="mt-4 space-y-3">{attempts.map((attempt, index) => <li key={`${attempt.channel_id}-${index}`} className="rounded-lg border border-border p-4">
        <div className="flex flex-wrap items-center justify-between gap-2"><div className="flex min-w-0 items-center gap-2"><span className="text-xs tabular-nums text-muted-foreground">{String(index + 1).padStart(2, '0')}</span><span className="break-all text-sm font-medium">{attempt.channel_name ?? '已删除渠道'}</span></div><Badge tone={attempt.outcome === 'success' ? 'success' : attempt.outcome === 'pending' ? 'neutral' : 'warning'}>{outcomes[attempt.outcome]}{attempt.status != null && ` · ${attempt.status}`}</Badge></div>
        <p className="mt-2 text-xs tabular-nums text-muted-foreground">请求开始后 {ms(attempt.started_ms)} 发起{attempt.outcome !== 'pending' && ` · 持续 ${ms(attempt.duration_ms)}`}</p>
        <dl className="mt-2"><Timing label="发出 → 响应头" value={attempt.headers_ms} />{detail.stream && <Timing label="发出 → 首个输出" value={attempt.first_token_ms} />}</dl>
      </li>)}</ol>}
    </section>

    <section aria-labelledby="request-usage-title"><h3 id="request-usage-title" className="font-semibold">用量与费用</h3><dl className="mt-3 grid grid-cols-2 gap-4 text-sm">{[['输入 Token', tokens(detail.input_tokens)], ['缓存 Token', tokens(detail.cached_tokens)], ['输出 Token', tokens(detail.output_tokens)], ['费用', detail.cost_usd != null ? `$${detail.cost_usd.toFixed(6)}` : '—']].map(([label, value]) => <div key={label}><dt className="text-xs text-muted-foreground">{label}</dt><dd className="mt-1 font-medium tabular-nums">{value}</dd></div>)}</dl></section>
    <div className="border-t border-border pt-4"><p className="text-xs text-muted-foreground">请求记录 ID</p><p className="mt-1 select-all break-all font-mono text-xs text-muted-foreground">{detail.id}</p></div>
  </div>
}

export function UsageRequestDetails({ requestId, accountId, onClose, returnFocus }: { requestId: string | null; accountId: string; onClose: () => void; returnFocus: () => void }) {
  const query = useQuery({ queryKey: ['usage-request', accountId, requestId], enabled: !!requestId,
    queryFn: ({ signal }) => api<UsageRequestDetail>(`/usage/events/${encodeURIComponent(requestId!)}`, { signal }),
    refetchInterval: query => query.state.data?.request_diagnostics?.completion === null ? 3000 : false,
  })
  return <Sheet open={!!requestId} onOpenChange={open => { if (!open) onClose() }}><SheetContent side="right" showCloseButton={false} className="w-full gap-0 p-0 sm:max-w-[640px]" onCloseAutoFocus={event => { event.preventDefault(); returnFocus() }}>
    <SheetHeader className="shrink-0 gap-2 border-b border-border px-6 py-5 pr-16"><SheetTitle>请求详情</SheetTitle><SheetDescription>查看耗时分解与渠道尝试过程</SheetDescription><SheetClose asChild><Button variant="ghost" size="icon" className="absolute right-4 top-4" aria-label="关闭请求详情"><X aria-hidden="true" /></Button></SheetClose></SheetHeader>
    <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-5 py-5 sm:px-6">
      <ErrorBox error={query.error} retry={() => { void query.refetch() }} />
      {query.isPending ? <Loading /> : query.data && <Details detail={query.data} />}
    </div>
    <div className="flex shrink-0 justify-end border-t border-border px-6 py-3"><Button variant="ghost" size="sm" disabled={query.isFetching} onClick={() => { void query.refetch() }}><RefreshCw aria-hidden="true" />{query.isFetching ? '更新中…' : '刷新详情'}</Button></div>
  </SheetContent></Sheet>
}
