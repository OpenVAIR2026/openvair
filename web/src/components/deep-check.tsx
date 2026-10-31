import { useEffect, useState } from 'react'
import { useMutation, useQuery } from '@tanstack/react-query'
import { ShieldAlert } from 'lucide-react'
import { api, jsonBody } from '../lib/api'
import { queryClient, useUser } from '../lib/queries'
import { confidenceOf, invalidReasons, tierCapability, verdictOf, type DeepCheckEvidence, type DeepCheckJob, type DeepCheckOptions } from '../lib/authenticity'
import { Badge, Choice, ErrorBox, Field, Loading } from './common'
import { Button } from './ui/button'
import { Checkbox } from './ui/checkbox'
import { Input } from './ui/input'

const active = (job: DeepCheckJob) => job.status === 'queued' || job.status === 'running'
const usd = (value: number) => value < 0.01 ? `$${value.toFixed(4)}` : `$${value.toFixed(2)}`

/** Starts a deep check of one monitor; results are shown in the monitor's details. */
export function DeepCheckForm({ monitorId, defaultClaim, onStarted, onCancel }: { monitorId: string; defaultClaim?: string | null; onStarted: (jobId: string) => void; onCancel: () => void }) {
  const user = useUser()
  const options = useQuery({ queryKey: ['deep-check-options', user.account_id, monitorId], queryFn: ({ signal }) => api<DeepCheckOptions>(`/quality-monitors/${monitorId}/deep-check`, { signal }) })
  const [claimed, setClaimed] = useState('')
  const [samples, setSamples] = useState(32)
  const [earlyStop, setEarlyStop] = useState(true)
  const [concurrency, setConcurrency] = useState(2)
  const [acknowledged, setAcknowledged] = useState(false)
  useEffect(() => { const preferred = defaultClaim && options.data?.models.some(m => m.model === defaultClaim) ? defaultClaim : options.data?.claimed_default; if (!claimed && preferred) setClaimed(preferred) }, [claimed, defaultClaim, options.data])
  const start = useMutation({
    mutationFn: () => api<{ job_id: string }>(`/quality-monitors/${monitorId}/deep-check`, jsonBody({ claimed_model: claimed, samples, early_stop: earlyStop, concurrency, acknowledge_risk: acknowledged })),
    onSuccess: async ({ job_id }) => {
      await Promise.all([queryClient.invalidateQueries({ queryKey: ['deep-checks', user.account_id, monitorId] }), queryClient.invalidateQueries({ queryKey: ['deep-check-options', user.account_id, monitorId] }), queryClient.invalidateQueries({ queryKey: ['quality-monitors', user.account_id] }), queryClient.invalidateQueries({ queryKey: ['authenticity-history', user.account_id] })])
      onStarted(job_id)
    },
  })
  if (options.isPending) return <Loading />
  if (!options.data) return <ErrorBox error={options.error} retry={() => void options.refetch()} />
  const data = options.data
  const model = data.models.find(m => m.model === claimed)
  const valid = !!claimed && samples >= data.min_samples && samples <= data.max_samples && (data.acknowledged || acknowledged)
  if (!data.models.length) return <p className="text-sm leading-6 text-muted-foreground">系统还没有启用的模型基准，暂时无法进行真伪检测。请联系平台管理员在「模型基准」中导入并启用基准。</p>
  return <form className="grid gap-5" onSubmit={e => { e.preventDefault(); if (valid) start.mutate() }}>
    <Field label="申报模型" hint={`渠道上游模型：${data.upstream_model}。按渠道声称提供的模型选择。`}>{id => <Choice id={id} name="claimed_model" value={claimed} onChange={setClaimed} options={data.models.map(m => ({ value: m.model, label: m.model }))} />}</Field>
    <fieldset className="grid gap-2"><legend className="mb-2 text-sm font-medium">检测档位</legend>
      {data.tiers.map((tier, index) => <label key={tier} className={`flex min-h-11 cursor-pointer items-start gap-3 rounded-lg border p-3 text-sm ${samples === tier ? 'border-primary/50 bg-primary/5' : 'border-border'}`}>
        <input type="radio" name="tier" className="mt-1" checked={samples === tier} onChange={() => setSamples(tier)} />
        <span className="grid gap-1"><span className="font-medium">{['快速', '标准', '深入'][index] ?? '档位'} · 最多 {tier} 个样本{index === 1 ? '（推荐）' : ''}</span><span className="text-xs text-muted-foreground">{tierCapability(tier, model)}{model?.per_sample_cost_usd != null ? ` · 预计最多 ${usd(model.per_sample_cost_usd * tier)}` : ''}</span></span>
      </label>)}
    </fieldset>
    <details className="rounded-lg border border-border p-4"><summary className="cursor-pointer text-sm font-medium focus-visible:outline-2 focus-visible:outline-ring">高级设置</summary><div className="mt-4 grid gap-4">
      <Field label="样本数" hint={`${data.min_samples}–${data.max_samples}。${samples < 16 ? '少于 16 个样本可能无法识别同家族降级。' : tierCapability(samples, model)}`}>{id => <Input id={id} name="samples" type="number" min={data.min_samples} max={data.max_samples} value={samples} onChange={e => setSamples(Number(e.target.value))} />}</Field>
      <label className="flex min-h-11 items-center gap-3 text-sm"><Checkbox checked={earlyStop} onCheckedChange={checked => setEarlyStop(checked === true)} />证据足够时提前停止（节省费用）</label>
      <Field label="并发" hint="遇到中转站限流时调低。">{id => <Choice id={id} name="concurrency" value={String(concurrency)} onChange={value => setConcurrency(Number(value))} options={[1, 2, 3, 4].map(n => ({ value: String(n), label: `${n}` }))} />}</Field>
    </div></details>
    <p className="text-xs leading-5 text-muted-foreground">检测请求按渠道自身计费{model?.per_sample_cost_usd != null ? `，按官方价格估算最多 ${usd(model.per_sample_cost_usd * samples)}` : '，该模型暂无价格，无法估算'}。{data.daily_sample_limit ? `每日样本额度 ${data.daily_sample_limit} 个。` : ''}结论只记录，不影响路由。</p>
    {!data.acknowledged && <div className="grid gap-3 rounded-lg border border-warning/40 bg-warning/5 p-4 text-sm">
      <p className="flex items-center gap-2 font-medium text-warning"><ShieldAlert aria-hidden="true" className="size-4" />首次检测前请确认</p>
      <ul className="grid list-disc gap-1 pl-5 text-xs leading-5 text-muted-foreground"><li>检测请求可能被中转站识别；部分中转站可能因此对该 Key 区别对待，甚至限制或封禁该 Key。</li><li>检测费用按该渠道的计费由你承担。</li><li>VAIR Key 渠道的检测请求计入来源 Key 的额度。</li></ul>
      <label className="flex min-h-11 items-center gap-3 text-sm"><Checkbox checked={acknowledged} onCheckedChange={checked => setAcknowledged(checked === true)} />我已了解以上风险</label>
    </div>}
    <ErrorBox error={start.error} />
    <div className="flex justify-end gap-2"><Button type="button" variant="outline" disabled={start.isPending} onClick={onCancel}>取消</Button><Button type="submit" disabled={!valid || start.isPending}>{start.isPending ? '提交中…' : '开始检测'}</Button></div>
  </form>
}

/** Checks of one monitor, polled while one is in progress. */
export function useDeepChecks(monitorId: string) {
  const user = useUser()
  return useQuery({ queryKey: ['deep-checks', user.account_id, monitorId], queryFn: ({ signal }) => api<DeepCheckJob[]>(`/quality-monitors/${monitorId}/deep-checks`, { signal }), refetchInterval: query => query.state.data?.some(active) ? 2000 : false })
}

export function Report({ job }: { job: DeepCheckJob }) {
  const verdict = verdictOf(job.verdict)
  const evidence: DeepCheckEvidence | null = job.evidence
  if (active(job)) return <div role="status" className="grid gap-2 rounded-lg border border-primary/30 bg-primary/5 p-4 text-sm">
    <p className="font-medium text-primary">{job.status === 'queued' ? '检测排队中…' : '检测进行中…'}</p>
    <p className="text-xs tabular-nums text-muted-foreground">已处理 {job.completed_samples}/{job.target_samples ?? '—'} · 有效 {job.valid_samples}</p>
    <div className="h-1.5 overflow-hidden rounded bg-muted"><div className="h-full bg-primary transition-all" style={{ width: `${Math.min(100, (job.completed_samples / Math.max(1, job.target_samples ?? 1)) * 100)}%` }} /></div>
  </div>
  if (!verdict || !evidence) return <div className="rounded-lg border border-border p-4 text-sm text-muted-foreground">检测{job.status === 'cancelled' ? '已取消：监控或渠道配置在检测期间发生了变化' : '未完成'}{job.error_code ? `（${job.error_code}）` : ''}。</div>
  const invalid = Object.entries(evidence.samples?.invalid ?? {})
  return <div className="grid gap-3 rounded-lg border border-border p-4 text-sm">
    <div className="flex flex-wrap items-center gap-3"><Badge tone={verdict.tone}>{verdict.label}</Badge><span className="break-all text-xs text-muted-foreground">申报 {evidence.claimed}{evidence.best_model && evidence.best_model !== evidence.claimed ? ` · 最像 ${evidence.best_model}` : ''}</span></div>
    <p className="text-xs leading-5 text-muted-foreground">{verdict.hint}</p>
    {!!evidence.reasons.length && <ul className="grid list-disc gap-1 pl-5 text-xs leading-5">{evidence.reasons.map(reason => <li key={reason}>{reason}</li>)}</ul>}
    <Confidence evidence={evidence} />
    <dl className="grid grid-cols-2 gap-x-4 gap-y-2 text-xs">
      {job.source === 'scheduled' ? <><dt className="text-muted-foreground">判定样本</dt><dd className="tabular-nums">窗口内 {evidence.valid_samples} 个（本轮有效 {evidence.samples?.valid ?? job.valid_samples} / {evidence.samples?.completed ?? job.completed_samples}）</dd></>
        : <><dt className="text-muted-foreground">有效样本</dt><dd className="tabular-nums">{evidence.valid_samples} / {evidence.samples?.completed ?? job.completed_samples}{job.stopped_early ? '（证据足够，提前结束）' : ''}</dd></>}
      {evidence.tokenizer && <><dt className="text-muted-foreground">Tokenizer 差分</dt><dd className="tabular-nums">实测 {evidence.tokenizer.observed_diff} · 基准 {evidence.tokenizer.claimed_diff ?? '—'}{evidence.tokenizer.consistent === false ? '（不一致）' : evidence.tokenizer.consistent ? '（一致）' : ''}</dd></>}
      {evidence.usage && <><dt className="text-muted-foreground">消耗 Token</dt><dd className="tabular-nums">输入 {evidence.usage.input_tokens.toLocaleString()} · 输出 {evidence.usage.output_tokens.toLocaleString()}</dd></>}
      {evidence.baseline && <><dt className="text-muted-foreground">基准</dt><dd className="tabular-nums">修订 {evidence.baseline.revision} · 提示词池 v{evidence.baseline.pool_version}{evidence.reasoning_mode === 'default' ? ' · 模型默认推理' : ''}</dd></>}
    </dl>
    {!!invalid.length && <p className="text-xs text-muted-foreground">无效样本：{invalid.map(([reason, count]) => `${invalidReasons[reason] ?? reason} ${count}`).join('，')}</p>}
    {evidence.stop_reason && <p className="text-xs text-warning">{evidence.stop_reason}</p>}
    {evidence.reasoning_param_rejected && <p className="text-xs text-muted-foreground">上游不接受关闭推理的参数，已按默认设置请求。</p>}
    {!!evidence.comparisons?.length && <details><summary className="cursor-pointer text-xs font-medium">与其他候选模型的比较</summary><table className="mt-2 w-full text-left text-xs"><thead className="text-muted-foreground"><tr><th className="py-1 font-medium">候选模型</th><th className="py-1 font-medium">厂商</th><th className="py-1 font-medium">Tokenizer</th><th className="py-1 text-right font-medium">倾向（z）</th></tr></thead><tbody>
      {evidence.comparisons.map(c => <tr key={c.model} className="border-t border-border"><td className="break-all py-1">{c.model}</td><td className="py-1">{c.family}{c.same_family ? '（同家族）' : ''}</td><td className={`py-1 ${c.token_consistent ? 'text-primary' : 'text-muted-foreground'}`}>{c.token_consistent == null ? '—' : c.token_consistent ? '与实测一致' : '不一致'}</td><td className={`py-1 text-right tabular-nums ${c.z < -1.645 ? 'text-warning' : ''}`}>{c.z.toFixed(1)}</td></tr>)}
    </tbody></table><p className="mt-2 text-xs text-muted-foreground">正值表示样本更像申报模型，负值表示更像该候选模型。Tokenizer 一列是该候选模型的基准计数与本次实测是否一致。</p></details>}
  </div>
}

/** The statistical certainty of a verdict, or why there is none. */
export function Confidence({ evidence }: { evidence: DeepCheckEvidence }) {
  const confidence = confidenceOf(evidence)
  if (!confidence) return evidence.verdict === 'insufficient' || evidence.verdict === 'ambiguous' ? <p className="text-xs text-muted-foreground">置信度：证据不足以下结论。</p> : null
  return <div className="rounded-md bg-muted/40 px-3 py-2 text-xs leading-5"><p><span className="font-medium">置信度 {confidence.value}</span><span className="text-muted-foreground"> · {confidence.detail}</span></p><p className="text-muted-foreground">按样本差异的单侧检验估算，只反映本次样本的把握；样本越多越可靠，也不能保证渠道从未偷换。</p></div>
}

export function VerdictBadge({ verdict }: { verdict: string | null }) {
  const v = verdictOf(verdict)
  return v ? <Badge tone={v.tone}>{v.label}</Badge> : <span className="text-xs text-muted-foreground">未检测</span>
}

