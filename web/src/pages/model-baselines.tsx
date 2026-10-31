import { useEffect, useRef, useState, type Dispatch, type KeyboardEvent, type SetStateAction } from 'react'
import { useMutation, useQuery } from '@tanstack/react-query'
import { Download, FileUp, Plus, Send } from 'lucide-react'
import { api, ApiError, formatDate, formatShortDate, jsonBody } from '../lib/api'
import { queryClient, useModelCatalog, useUser } from '../lib/queries'
import { invalidReasons, neededLabel, reasoningModes, type OpenRouterLookup, type ReasoningMode, type BaselineBatch, type BaselineEvaluation, type BaselineModel, type BaselineOverview, type BaselinePlan, type BaselineVersion, type KindCount, type PoolVersion, type ProbeTemplate, type SampleCounts, type VersionProgress } from '../lib/authenticity'
import { Badge, Choice, Confirm, Empty, ErrorBox, Field, FormDialog, FormSheet, Loading, PageHeader, Panel } from '../components/common'
import { Button } from '../components/ui/button'
import { Checkbox } from '../components/ui/checkbox'
import { Input } from '../components/ui/input'
import { SuperAdminGuard } from './platform'

type Tab = 'models' | 'imports' | 'pool'
const tabs: { value: Tab; label: string }[] = [{ value: 'models', label: '基准模型' }, { value: 'imports', label: '导入记录' }, { value: 'pool', label: '提示词池' }]
const versionStatus = { active: '启用中', draft: '草稿', retired: '已停用' } as const

/** Valid/total of one probe kind in a batch. */
const kindCount = (count?: KindCount) => count ? `${count.valid}/${count.total}` : '—'
/** "指纹 199/200 有效 · Tokenizer 9/9 有效", leaving out a kind the batch has no samples of. */
const batchCounts = (counts: SampleCounts) => [counts.fingerprint && `指纹 ${kindCount(counts.fingerprint)} 有效`, counts.token && `Tokenizer ${kindCount(counts.token)} 有效`].filter(Boolean).join(' · ') || '没有样本'
const invalidText = (invalid: Record<string, number>) => Object.entries(invalid).map(([reason, count]) => `${count} 个${invalidReasons[reason] ?? reason}`).join('，')

/** The version's fingerprint samples and, per template, its Tokenizer samples against the target. */
function SampleProgress({ fingerprint, target, progress }: { fingerprint: number; target?: number; progress: VersionProgress | null }) {
  return <>指纹样本 {fingerprint}{target != null && `/${target}`}{progress && <> · Tokenizer {progress.token_counted}/{progress.token_target}</>}</>
}
const textareaClass = 'min-h-28 w-full rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-xs outline-none focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 dark:bg-input/30'

const refresh = () => Promise.all(['baselines', 'baseline-imports', 'baseline-versions', 'probe-templates'].map(name => queryClient.invalidateQueries({ queryKey: [name] })))

export function PlatformBaselinesPage() {
  const [tab, setTab] = useState<Tab>('models')
  // Roving tabindex with arrow / Home / End keys, per the WAI-ARIA tabs pattern.
  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    const index = tabs.findIndex(t => t.value === tab)
    const next = event.key === 'ArrowRight' ? (index + 1) % tabs.length : event.key === 'ArrowLeft' ? (index - 1 + tabs.length) % tabs.length : event.key === 'Home' ? 0 : event.key === 'End' ? tabs.length - 1 : -1
    if (next < 0) return
    event.preventDefault()
    setTab(tabs[next].value)
    document.getElementById(`baseline-tab-${tabs[next].value}`)?.focus()
  }
  return <SuperAdminGuard>
    <PageHeader title="模型基准" description="维护真伪检测使用的模型指纹基准。基准在网络合规的机器上按采集计划采集，再增量导入；合并前会校验、检查分布一致性并评估检测能力。" />
    <div role="tablist" aria-label="模型基准" className="grid grid-cols-3 rounded-xl border border-border bg-card p-1">
      {tabs.map(t => <button key={t.value} id={`baseline-tab-${t.value}`} type="button" role="tab" aria-selected={tab === t.value} aria-controls={`baseline-panel-${t.value}`} tabIndex={tab === t.value ? 0 : -1} onKeyDown={onKeyDown} onClick={() => setTab(t.value)} className={`rounded-lg px-4 py-3 text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring ${tab === t.value ? 'bg-primary/12 text-primary' : 'text-muted-foreground hover:bg-muted/40 hover:text-foreground'}`}>{t.label}</button>)}
    </div>
    <div role="tabpanel" id={`baseline-panel-${tab}`} aria-labelledby={`baseline-tab-${tab}`} className="mt-5">
      {tab === 'models' ? <ModelsTab /> : tab === 'imports' ? <ImportsTab /> : <PoolTab />}
    </div>
  </SuperAdminGuard>
}

function useOverview() {
  const user = useUser()
  return useQuery({ queryKey: ['baselines', user.account_id], queryFn: ({ signal }) => api<BaselineOverview>('/platform/model-baselines', { signal }) })
}

function Capability({ evaluation }: { evaluation: BaselineEvaluation | null | undefined }) {
  if (!evaluation) return <span className="text-xs text-muted-foreground">评估中或样本不足</span>
  if (!evaluation.pairs.length) return <span className="text-xs text-muted-foreground">没有其他启用的基准可比较</span>
  const siblings = evaluation.pairs.some(p => p.same_family)
  const cross = evaluation.pairs.some(p => !p.same_family)
  return <div className="grid gap-1 text-xs">{cross && <span>跨厂商：{neededLabel(evaluation.cross_family)}</span>}{siblings && <span>同家族：{neededLabel(evaluation.same_family)}</span>}</div>
}

// ---------------------------------------------------------------- models

type TargetEditor = { model: string; isNew: boolean; family: string; openrouter_model: string; provider_only: string; target_samples: number; token_samples: number; reasoning: ReasoningMode }

function ModelsTab() {
  const overview = useOverview()
  const catalog = useModelCatalog()
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [editor, setEditor] = useState<TargetEditor | null>(null)
  const [versionsOf, setVersionsOf] = useState<string | null>(null)
  const [plan, setPlan] = useState<BaselinePlan | null>(null)
  const save = useMutation({
    mutationFn: (value: TargetEditor) => api(`/platform/model-baselines/models/${encodeURIComponent(value.model.trim())}`, jsonBody({ family: value.family, openrouter_model: value.openrouter_model, provider_only: value.provider_only, target_samples: value.target_samples, token_samples: value.token_samples, reasoning: value.reasoning }, 'PUT')),
    onSuccess: async () => { setEditor(null); await refresh() },
  })
  const createPlan = useMutation({ mutationFn: (models: string[]) => api<BaselinePlan>('/platform/model-baselines/plans', jsonBody({ models })), onSuccess: setPlan })
  if (overview.isPending) return <Loading />
  if (!overview.data) return <ErrorBox error={overview.error} retry={() => void overview.refetch()} />
  const models = overview.data.models
  const concrete = (catalog.data ?? []).filter(item => !item.pattern.includes('*') && !models.some(m => m.model === item.pattern))
  const toggle = (model: string) => setSelected(current => { const next = new Set(current); if (next.has(model)) next.delete(model); else next.add(model); return next })
  const openNew = () => { save.reset(); setEditor({ model: '', isNew: true, family: '', openrouter_model: '', provider_only: '', target_samples: 200, token_samples: 9, reasoning: 'off' }) }
  return <>
    <div className="flex flex-wrap items-center justify-between gap-3">
      <p className="text-xs text-muted-foreground">提示词池 {overview.data.pool_version ? `v${overview.data.pool_version}` : '尚未发布'} · 取值范围 1–{overview.data.range_max}</p>
      <div className="flex gap-2"><Button variant="outline" onClick={openNew}><Plus aria-hidden="true" className="size-4" />添加模型</Button><Button disabled={!selected.size || createPlan.isPending} onClick={() => createPlan.mutate([...selected])}><Send aria-hidden="true" className="size-4" />{createPlan.isPending ? '生成中…' : `生成采集计划${selected.size ? `（${selected.size}）` : ''}`}</Button></div>
    </div>
    <div className="mt-3"><ErrorBox error={createPlan.error} /></div>
    {!models.length ? <Panel className="mt-4"><Empty title="还没有基准模型" description="先在模型管理中添加具体模型，再在这里配置它的 OpenRouter 模型 ID 和锁定的官方供应商。" action={<Button onClick={openNew}>添加模型</Button>} /></Panel> : <Panel className="mt-4 overflow-hidden"><div className="overflow-x-auto"><table className="w-full min-w-[1000px] text-left text-sm">
      <thead className="bg-muted/30 text-xs text-muted-foreground"><tr><th className="w-10 px-4 py-3"><span className="sr-only">选择</span></th>{['模型', '采集来源', '启用版本', '检测能力（95%）', '状态', '操作'].map(title => <th key={title} className="px-4 py-3 font-medium">{title}</th>)}</tr></thead>
      <tbody>{models.map(m => <ModelRow key={m.model} model={m} checked={selected.has(m.model)} onToggle={() => toggle(m.model)} onEdit={() => { save.reset(); setEditor({ ...m, isNew: false }) }} onVersions={() => setVersionsOf(m.model)} />)}</tbody>
    </table></div></Panel>}
    <FormSheet open={!!editor} onOpenChange={open => { if (!open && !save.isPending) setEditor(null) }} title={editor?.isNew ? '添加基准模型' : '编辑基准模型'} description="采集时会锁定官方供应商并关闭 fallback，其他供应商处理的样本会被作废。">
      {editor && <TargetForm editor={editor} setEditor={setEditor} concrete={concrete.map(item => ({ pattern: item.pattern, label: item.label }))} pending={save.isPending} error={save.error} onSubmit={() => save.mutate(editor)} />}
    </FormSheet>
    <VersionsSheet model={versionsOf} target={models.find(m => m.model === versionsOf)?.target_samples} onClose={() => setVersionsOf(null)} />
    <PlanDialog plan={plan} onClose={() => { setPlan(null); setSelected(new Set()) }} />
  </>
}

function TargetForm({ editor, setEditor, concrete, pending, error, onSubmit }: { editor: TargetEditor; setEditor: Dispatch<SetStateAction<TargetEditor | null>>; concrete: { pattern: string; label: string }[]; pending: boolean; error: Error | null; onSubmit: () => void }) {
  const user = useUser()
  const model = editor.model.trim()
  const [chosen, setChosen] = useState<string | null>(null)
  // New models come from the price table; their OpenRouter settings are looked up, not typed.
  const known = editor.isNew && concrete.some(item => item.pattern === model)
  const lookup = useQuery({ queryKey: ['baseline-openrouter', user.account_id, model, chosen], queryFn: ({ signal }) => api<OpenRouterLookup>(`/platform/model-baselines/openrouter?model=${encodeURIComponent(model)}${chosen ? `&openrouter_model=${encodeURIComponent(chosen)}` : ''}`, { signal }), enabled: known, staleTime: 10 * 60 * 1000 })
  useEffect(() => {
    const found = lookup.data
    if (!found) return
    setEditor(current => current && { ...current, family: found.family ?? current.family, openrouter_model: found.openrouter_model ?? current.openrouter_model, provider_only: found.provider_only ?? '' })
  }, [lookup.data, setEditor])
  const set = (patch: Partial<TargetEditor>) => setEditor(current => current && { ...current, ...patch })
  const candidate = lookup.data?.candidates.find(c => c.id === editor.openrouter_model)
  const providers = lookup.data?.providers ?? []
  const lookupHint = !editor.isNew ? undefined : !model ? '从价格表（模型管理）中选择，系统会自动查询 OpenRouter 上对应的模型和供应商。' : !known ? '价格表中没有这个模型，请先在模型管理中添加。' : lookup.isFetching ? '正在查询 OpenRouter…' : lookup.error ? `${lookup.error.message}` : lookup.data && !lookup.data.openrouter_model ? 'OpenRouter 上没有找到对应的模型，请手动填写下面的信息。' : '已根据 OpenRouter 自动填写，请检查后保存。'
  return <form className="grid gap-5" onSubmit={e => { e.preventDefault(); onSubmit() }}>
    {editor.isNew ? <Field label="模型" hint={lookupHint}>{id => <><Input id={id} name="model" required list="baseline-catalog" autoComplete="off" placeholder="搜索价格表中的模型…" value={editor.model} onChange={e => { setChosen(null); set({ model: e.target.value }) }} /><datalist id="baseline-catalog">{concrete.map(item => <option key={item.pattern} value={item.pattern}>{item.label}</option>)}</datalist></>}</Field> : <p className="break-all font-medium">{editor.model}</p>}
    {(lookup.data?.candidates.length ?? 0) > 1 ? <Field label="OpenRouter 模型" hint="价格表名称对应了多个 OpenRouter 模型，请选择要采集的那个。">{id => <Choice id={id} name="openrouter_model" value={editor.openrouter_model} onChange={value => { setChosen(value); set({ openrouter_model: value }) }} options={lookup.data!.candidates.map(c => ({ value: c.id, label: c.id }))} />}</Field>
      : <Field label="OpenRouter 模型 ID" hint="例如 anthropic/claude-opus-5.5。">{id => <Input id={id} name="openrouter_model" required value={editor.openrouter_model} onChange={e => set({ openrouter_model: e.target.value })} />}</Field>}
    {providers.length ? <Field label="锁定供应商" hint="采集只走这一家供应商，其他供应商处理的样本会被作废。默认选官方供应商。">{id => <Choice id={id} name="provider_only" value={editor.provider_only} onChange={value => { if (value) set({ provider_only: value }) }} options={providers.map(p => ({ value: p.slug, label: `${p.name ?? p.slug}（${p.slug}）${p.quantization && p.quantization !== 'unknown' ? ` · ${p.quantization}` : ''}` }))} />}</Field>
      : <Field label="锁定供应商" hint="OpenRouter 供应商标识，例如 anthropic、openai、alibaba。">{id => <Input id={id} name="provider_only" required value={editor.provider_only} onChange={e => set({ provider_only: e.target.value })} />}</Field>}
    <Field label="厂商" hint="用于区分跨厂商替换和同家族降级，例如 anthropic、openai、qwen。">{id => <Input id={id} name="family" required value={editor.family} onChange={e => set({ family: e.target.value })} />}</Field>
    <Field label="推理模式" hint={`能关闭推理的模型选「关闭推理」，避免推理改变数字分布并增加成本。OpenRouter 返回「Reasoning is mandatory」的模型（例如 Claude 5.5、GPT-6.1）选「模型默认」。${candidate?.supports_reasoning ? '这个模型支持推理，采集前可以用 ping --reasoning 确认能否关闭。' : ''}修改后需要新建基准版本。`}>{id => <Choice id={id} name="reasoning" value={editor.reasoning} onChange={value => set({ reasoning: value as ReasoningMode })} options={reasoningModes} />}</Field>
    <Field label="目标指纹样本数" hint="20–5000。默认 200；评估显示同家族型号区分不开时再调高并增量补采。">{id => <Input id={id} name="target_samples" type="number" min={20} max={5000} required value={editor.target_samples} onChange={e => set({ target_samples: Number(e.target.value) })} />}</Field>
    <Field label="Tokenizer 样本数" hint="0–1000，平均分配到提示词池中的 Tokenizer 模板。Token 计数是确定的，填模板数 × 3 即可（3 个模板填 9）。">{id => <Input id={id} name="token_samples" type="number" min={0} max={1000} required value={editor.token_samples} onChange={e => set({ token_samples: Number(e.target.value) })} />}</Field>
    <ErrorBox error={error} />
    <div className="flex justify-end gap-2"><Button type="button" variant="outline" onClick={() => setEditor(null)}>取消</Button><Button type="submit" disabled={pending || (editor.isNew && !known) || !editor.provider_only}>{pending ? '保存中…' : '保存'}</Button></div>
  </form>
}

function ModelRow({ model: m, checked, onToggle, onEdit, onVersions }: { model: BaselineModel; checked: boolean; onToggle: () => void; onEdit: () => void; onVersions: () => void }) {
  const version = m.active ?? m.draft
  const evaluation = version?.evaluation
  return <tr className="border-t border-border hover:bg-muted/30">
    <td className="px-4 py-4"><Checkbox aria-label={`选择 ${m.model}`} checked={checked} onCheckedChange={onToggle} /></td>
    <td className="max-w-56 px-4 py-4"><div className="break-all font-medium">{m.model}</div><div className="mt-1 text-xs text-muted-foreground">{m.family}</div></td>
    <td className="max-w-56 px-4 py-4 text-xs"><div className="break-all">{m.openrouter_model}</div><div className="mt-1 text-muted-foreground">锁定 {m.provider_only} · {m.reasoning === 'off' ? '关闭推理' : '模型默认推理'}</div></td>
    <td className="px-4 py-4 text-xs tabular-nums">{m.active ? <><div>修订 {m.active.revision} · 池 v{m.active.pool_version}</div><div className="mt-1 text-muted-foreground"><SampleProgress fingerprint={m.active.valid_samples} target={m.target_samples} progress={m.active.progress} /></div></> : <span className="text-muted-foreground">未启用</span>}</td>
    <td className="px-4 py-4"><Capability evaluation={evaluation} /></td>
    <td className="px-4 py-4"><div className="flex flex-wrap gap-1">{!m.active && <Badge tone="warning">未启用</Badge>}{m.draft && <Badge>草稿 指纹样本 {m.draft.valid_samples}</Badge>}{m.pending_batches > 0 && <Badge tone="warning">{m.pending_batches} 个批次待处理</Badge>}{m.active && m.active.valid_samples < m.target_samples && <Badge>待补采指纹 {m.target_samples - m.active.valid_samples}</Badge>}{!!m.active?.progress?.token_missing && <Badge>待补采 Tokenizer {m.active.progress.token_missing}</Badge>}</div>{evaluation?.recommendation && <p className="mt-1 max-w-56 text-xs text-warning">{evaluation.recommendation}</p>}</td>
    <td className="px-4 py-4"><div className="flex justify-end gap-1"><Button size="sm" variant="ghost" onClick={onVersions}>版本</Button><Button size="sm" variant="ghost" onClick={onEdit}>编辑</Button></div></td>
  </tr>
}

function VersionsSheet({ model, target, onClose }: { model: string | null; target?: number; onClose: () => void }) {
  const user = useUser()
  const versions = useQuery({ queryKey: ['baseline-versions', user.account_id, model], queryFn: ({ signal }) => api<BaselineVersion[]>(`/platform/model-baselines/models/${encodeURIComponent(model ?? '')}/versions`, { signal }), enabled: !!model })
  const [exclude, setExclude] = useState<string | null>(null)
  const activate = useMutation({ mutationFn: (id: string) => api(`/platform/model-baselines/versions/${id}/activate`, { method: 'POST' }), onSuccess: refresh })
  const exclusion = useMutation({ mutationFn: (id: string) => api(`/platform/model-baselines/batches/${id}/exclude`, { method: 'POST' }), onSuccess: async () => { setExclude(null); await refresh() } })
  return <FormSheet open={!!model} onOpenChange={open => { if (!open) onClose() }} title="基准版本" description={model ?? undefined}>
    <ErrorBox error={versions.error ?? activate.error} />
    {versions.isPending ? <Loading /> : !versions.data?.length ? <p className="text-sm text-muted-foreground">还没有版本。生成采集计划、采集并导入后会出现在这里。</p> : <div className="grid gap-4">{versions.data.map(v => <div key={v.id} className="grid gap-3 rounded-lg border border-border p-4 text-sm">
      <div className="flex flex-wrap items-center gap-2"><Badge tone={v.status === 'active' ? 'success' : v.status === 'draft' ? 'warning' : 'neutral'}>{versionStatus[v.status]}</Badge><span className="text-xs tabular-nums text-muted-foreground">修订 {v.revision} · 池 v{v.pool_version} · 锁定 {v.provider_only}</span>{v.status !== 'active' && <Button size="sm" variant="outline" className="ml-auto" disabled={activate.isPending} onClick={() => activate.mutate(v.id)}>{v.status === 'retired' ? '回退到此版本' : '启用'}</Button>}</div>
      <p className="text-xs tabular-nums"><SampleProgress fingerprint={v.valid_samples} target={target} progress={v.progress} />{v.progress && ` · ${v.progress.batches} 个批次`}</p>
      <Capability evaluation={v.evaluation} />
      {v.evaluated_at && <p className="text-xs text-muted-foreground">评估于 {formatDate(v.evaluated_at)}（修订 {v.evaluated_revision}{v.evaluated_revision !== v.revision ? '，正在重新评估' : ''}）</p>}
      {!!v.batches.length && <ul className="grid gap-1 text-xs">{v.batches.map(b => <li key={b.id} className="flex flex-wrap items-center gap-2"><span className="tabular-nums text-muted-foreground" title={formatDate(b.created_at)}>{formatShortDate(b.created_at)}</span><span className="tabular-nums">{batchCounts(b.counts)}</span>{b.excluded_at ? <Badge>已排除</Badge> : <Button size="sm" variant="ghost" className="ml-auto" onClick={() => { exclusion.reset(); setExclude(b.id) }}>排除</Button>}</li>)}</ul>}
    </div>)}</div>}
    <Confirm open={!!exclude} onOpenChange={open => { if (!open && !exclusion.isPending) setExclude(null) }} title="排除批次" description="排除后该批次的样本不再参与基准统计，版本修订号加一并重新评估。样本保留以便追溯。" pending={exclusion.isPending} error={exclusion.error} onConfirm={() => { if (exclude) exclusion.mutate(exclude) }} />
  </FormSheet>
}

function PlanDialog({ plan, onClose }: { plan: BaselinePlan | null; onClose: () => void }) {
  const [error, setError] = useState<Error | null>(null)
  const [downloading, setDownloading] = useState(false)
  const fileName = plan ? `plan-${plan.id.slice(0, 8)}.json` : ''
  async function download() {
    if (!plan) return
    setDownloading(true); setError(null)
    try {
      const file = await api<unknown>(`/platform/model-baselines/plans/${plan.id}/download`)
      const url = URL.createObjectURL(new Blob([JSON.stringify(file, null, 2)], { type: 'application/json' }))
      const link = document.createElement('a'); link.href = url; link.download = fileName; link.click(); URL.revokeObjectURL(url)
    } catch (e) { setError(e as Error) } finally { setDownloading(false) }
  }
  return <FormDialog open={!!plan} onOpenChange={open => { if (!open) onClose() }} title="采集计划" description="计划文件包含保密的提示词池，请像密钥一样保管，用完即删。下载会记入审计日志。">
    {plan && <div className="grid gap-4 text-sm">
      <table className="w-full text-left text-xs"><thead className="text-muted-foreground"><tr><th className="py-1 font-medium">模型</th><th className="py-1 text-right font-medium">指纹</th><th className="py-1 text-right font-medium">Tokenizer</th></tr></thead><tbody>
        {plan.items.map(item => <tr key={item.model} className="border-t border-border"><td className="break-all py-1">{item.model}<div className="text-muted-foreground">{item.openrouter_model}@{item.provider_only} · {item.reasoning === 'off' ? '关闭推理' : '模型默认推理'}{item.version_id ? ' · 增量补采' : ' · 新版本'}</div></td><td className="py-1 text-right tabular-nums">{item.fingerprint}</td><td className="py-1 text-right tabular-nums">{Object.values(item.token).reduce((a, b) => a + b, 0)}</td></tr>)}
      </tbody></table>
      <p className="text-xs text-muted-foreground">提示词池 v{plan.pool_version} · 按官方价格预估约 ${plan.estimated_cost_usd.toFixed(2)} · {formatDate(plan.expires_at)} 前有效</p>
      <ol className="grid list-decimal gap-2 pl-5 text-xs leading-5"><li>下载计划文件，复制到网络出口能访问 OpenRouter 官方供应商的机器上。</li><li>设置 <code>OPENROUTER_API_KEY</code> 后运行：<code className="mt-1 block break-all rounded bg-background p-2">python3 scripts/authenticity_spike.py collect --plan {fileName}</code></li><li>在「导入记录」中上传生成的 <code>bundles/*.jsonl</code>。</li></ol>
      <ErrorBox error={error} />
      <div className="flex justify-end"><Button disabled={downloading} onClick={() => void download()}><Download aria-hidden="true" className="size-4" />{downloading ? '下载中…' : '下载计划文件'}</Button></div>
    </div>}
  </FormDialog>
}

// ---------------------------------------------------------------- imports

function ImportsTab() {
  const user = useUser()
  const imports = useQuery({ queryKey: ['baseline-imports', user.account_id], queryFn: ({ signal }) => api<BaselineBatch[]>('/platform/model-baselines/imports', { signal }) })
  const input = useRef<HTMLInputElement>(null)
  const [uploading, setUploading] = useState(false)
  const [results, setResults] = useState<string[]>([])
  const [expanded, setExpanded] = useState<string | null>(null)
  const decide = useMutation({ mutationFn: ({ id, action }: { id: string; action: 'merge' | 'new-version' | 'reject' }) => api(`/platform/model-baselines/imports/${id}/${action}`, { method: 'POST' }), onSuccess: refresh })
  async function upload(files: FileList | null) {
    if (!files?.length) return
    setUploading(true); setResults([])
    const lines: string[] = []
    for (const file of Array.from(files)) {
      try {
        const batch = await api<BaselineBatch>('/platform/model-baselines/imports', { method: 'POST', headers: { 'content-type': 'application/x-ndjson' }, body: await file.text() })
        lines.push(`${file.name}：${batch.model}，${batchCounts(batch.counts)}`)
      } catch (e) { lines.push(`${file.name}：${e instanceof ApiError ? e.message : '上传失败'}`) }
    }
    setResults(lines); setUploading(false)
    if (input.current) input.current.value = ''
    await refresh()
  }
  return <>
    <Panel className="p-5"><div className="flex flex-wrap items-center gap-4"><div className="flex-1"><h2 className="text-sm font-semibold">上传导入包</h2><p className="mt-1 text-xs leading-5 text-muted-foreground">采集工具生成的 <code>bundles/*.jsonl</code>，可以一次选择多个。服务器会重新校验每个样本，并与现有版本比较分布。重复上传同一个包不会重复导入。</p></div>
      <input ref={input} type="file" accept=".jsonl,application/x-ndjson" multiple className="sr-only" id="baseline-upload" onChange={e => void upload(e.target.files)} />
      <Button asChild disabled={uploading}><label htmlFor="baseline-upload" className="cursor-pointer"><FileUp aria-hidden="true" className="size-4" />{uploading ? '上传中…' : '选择文件'}</label></Button></div>
      {!!results.length && <ul role="status" className="mt-3 grid gap-1 text-xs">{results.map(line => <li key={line} className="break-all">{line}</li>)}</ul>}
      <div className="mt-3"><ErrorBox error={decide.error} /></div>
    </Panel>
    {imports.isPending ? <Loading /> : !imports.data?.length ? <Panel className="mt-4"><Empty title="还没有导入记录" description="生成采集计划并在本地采集后，在这里上传导入包。" /></Panel> : <Panel className="mt-4 overflow-hidden"><div className="overflow-x-auto"><table className="w-full min-w-[1100px] text-left text-sm">
      <thead className="bg-muted/30 text-xs text-muted-foreground"><tr>{['时间', '模型', '指纹样本', 'Tokenizer', '所属版本', '分布一致性', '合并后检测能力', '状态', '操作'].map(title => <th key={title} className="px-4 py-3 font-medium">{title}</th>)}</tr></thead>
      <tbody>{imports.data.map(b => <BatchRow key={b.id} batch={b} open={expanded === b.id} toggle={() => setExpanded(expanded === b.id ? null : b.id)} pending={decide.isPending} onDecide={action => decide.mutate({ id: b.id, action })} />)}</tbody>
    </table></div></Panel>}
  </>
}

function BatchRow({ batch: b, open, toggle, pending, onDecide }: { batch: BaselineBatch; open: boolean; toggle: () => void; pending: boolean; onDecide: (action: 'merge' | 'new-version' | 'reject') => void }) {
  const drift = b.drift
  const canMerge = !!b.validation.compatible_version_id && !drift?.flagged
  // Decided by where the batch went: an admin may import a compatible batch as a new version.
  const kind = b.status === 'merged' ? (b.version_id === b.validation.compatible_version_id ? '增量补采' : '新版本')
    : b.status === 'pending' ? (canMerge ? `可合并到修订 ${b.compatible_revision ?? ''}` : '只能作为新版本') : null
  const cell = (count?: KindCount) => <td className="px-4 py-4 text-xs tabular-nums">{kindCount(count)}{count && !!Object.keys(count.invalid).length && <div className="mt-1 max-w-40 text-warning">{invalidText(count.invalid)}</div>}</td>
  return <>
    <tr className="border-t border-border">
      <td className="whitespace-nowrap px-4 py-4 text-xs tabular-nums text-muted-foreground" title={formatDate(b.created_at)}>{formatShortDate(b.created_at)}</td>
      <td className="max-w-48 break-all px-4 py-4">{b.model}<div className="mt-1 text-xs text-muted-foreground">池 v{b.manifest.pool_version}{kind && ` · ${kind}`}</div></td>
      {cell(b.counts.fingerprint)}{cell(b.counts.token)}
      <td className="px-4 py-4 text-xs tabular-nums">{b.version ? <>修订 {b.version.revision}（{versionStatus[b.version.status]}）<div className="mt-1 text-muted-foreground">当前指纹 {b.version.valid_samples}{b.target_samples != null && `/${b.target_samples}`}</div></> : <span className="text-muted-foreground">—</span>}</td>
      <td className="px-4 py-4">{!drift?.checked ? <span className="text-xs text-muted-foreground">无可比较版本</span> : drift.flagged ? <Badge tone="danger">分布变化</Badge> : <Badge tone="success">一致</Badge>}</td>
      <td className="px-4 py-4"><Capability evaluation={b.preview} /></td>
      <td className="px-4 py-4"><Badge tone={b.status === 'merged' ? (b.excluded_at ? 'neutral' : 'success') : b.status === 'rejected' ? 'neutral' : 'warning'}>{b.status === 'merged' ? (b.excluded_at ? '已排除' : '已合并') : b.status === 'rejected' ? '已拒绝' : '待处理'}</Badge></td>
      <td className="px-4 py-4"><div className="flex justify-end gap-1">{b.status === 'pending' && <>{canMerge && <Button size="sm" variant="outline" disabled={pending} onClick={() => onDecide('merge')}>合并</Button>}<Button size="sm" variant="ghost" disabled={pending} onClick={() => onDecide('new-version')}>作为新版本</Button><Button size="sm" variant="ghost" disabled={pending} onClick={() => onDecide('reject')}>拒绝</Button></>}<Button size="sm" variant="ghost" aria-expanded={open} onClick={toggle}>{open ? '收起' : '详情'}</Button></div></td>
    </tr>
    {open && <tr><td colSpan={9} className="border-t border-border bg-muted/20 p-4"><div className="grid gap-2 text-xs leading-5">
      <p>来源 {b.manifest.openrouter_model}@{b.manifest.provider_only} · 工具 {b.manifest.tool_version} · 采集 {b.manifest.started_at} – {b.manifest.finished_at}</p>
      {!!b.validation.duplicates && <p>重复样本 {b.validation.duplicates} 个，上传时已丢弃</p>}
      {drift?.checked && <p className="text-muted-foreground">分布差异 JSD {drift.jsd.toFixed(4)} · p = {drift.p_value.toFixed(3)}{drift.flagged ? '。与现有版本明显不同，模型可能已更新，只能作为新版本导入。' : ''}</p>}
      {b.status === 'pending' && !b.validation.compatible_version_id && <p className="text-muted-foreground">没有采集条件相同的现有版本，只能作为新版本导入。</p>}
      {b.preview?.recommendation && <p className="text-warning">{b.preview.recommendation}</p>}
    </div></td></tr>}
  </>
}

// ---------------------------------------------------------------- prompt pool

function PoolTab() {
  const user = useUser()
  const pool = useQuery({ queryKey: ['probe-templates', user.account_id], queryFn: ({ signal }) => api<{ templates: ProbeTemplate[]; versions: PoolVersion[] }>('/platform/probe-templates', { signal }) })
  const [adding, setAdding] = useState<{ probe_kind: string; content: string } | null>(null)
  const [retire, setRetire] = useState<ProbeTemplate | null>(null)
  const create = useMutation({ mutationFn: (value: { probe_kind: string; content: string }) => api('/platform/probe-templates', jsonBody(value)), onSuccess: async () => { setAdding(null); await refresh() } })
  const retirement = useMutation({ mutationFn: (id: string) => api(`/platform/probe-templates/${id}/retire`, { method: 'POST' }), onSuccess: async () => { setRetire(null); await refresh() } })
  const publish = useMutation({ mutationFn: () => api<{ version: number }>('/platform/probe-templates/publish', { method: 'POST' }), onSuccess: refresh })
  const defaults = useMutation({ mutationFn: () => api<{ added: number }>('/platform/probe-templates/defaults', { method: 'POST' }), onSuccess: refresh })
  if (pool.isPending) return <Loading />
  if (!pool.data) return <ErrorBox error={pool.error} retry={() => void pool.refetch()} />
  const latest = pool.data.versions[0]
  const inLatest = new Set(latest?.template_ids ?? [])
  const published = (kind: ProbeTemplate['probe_kind']) => pool.data.templates.filter(t => t.probe_kind === kind && inLatest.has(t.id)).length
  const unpublished = pool.data.templates.some(t => !t.retired_at && !inLatest.has(t.id)) || (latest?.template_ids ?? []).some(id => pool.data.templates.find(t => t.id === id)?.retired_at)
  return <>
    <Panel className="p-5"><div className="flex flex-wrap items-center gap-4"><div className="flex-1"><h2 className="text-sm font-semibold">{latest ? `当前版本 v${latest.version}：指纹模板 ${published('fingerprint')} 个，Tokenizer 模板 ${published('token')} 个` : '尚未发布'}</h2><p className="mt-1 text-xs leading-5 text-muted-foreground">模板加密保存，不出现在代码仓库和审计日志里。发布新版本后，已有基准仍按原版本使用；要用新模板检测，需要按新版本采集基准。{latest ? `发布于 ${formatDate(latest.published_at)}。` : ''}</p></div>
      <Button variant="outline" disabled={defaults.isPending} onClick={() => defaults.mutate()}>{defaults.isPending ? '导入中…' : '导入默认模板'}</Button>
      <Button variant="outline" onClick={() => { create.reset(); setAdding({ probe_kind: 'fingerprint', content: '' }) }}><Plus aria-hidden="true" className="size-4" />添加模板</Button>
      <Button disabled={!unpublished || publish.isPending} onClick={() => publish.mutate()}>{publish.isPending ? '发布中…' : '发布新版本'}</Button></div>
      {defaults.data && <p role="status" className="mt-3 text-xs text-primary">{defaults.data.added ? `已导入 ${defaults.data.added} 个默认模板，发布新版本后生效。` : '默认模板都已存在（包括已停用的），没有新增。'}</p>}
      {unpublished && <p className="mt-3 text-xs text-warning">有尚未发布的模板改动。</p>}
      {latest && <p className="mt-3 text-xs text-muted-foreground">建议基准模型的 Tokenizer 样本数设为 Tokenizer 模板数 × 3（当前 {published('token') * 3}）：Token 计数是确定的，每个模板采 3 次就够。</p>}
      <div className="mt-3"><ErrorBox error={publish.error ?? defaults.error} /></div>
    </Panel>
    {!pool.data.templates.length ? <Panel className="mt-4"><Empty title="还没有模板" description="可以先导入默认模板（10 个指纹模板、3 个 Tokenizer 模板），再按需补充自己的模板。至少需要 1 个指纹模板和 2 个 Tokenizer 模板才能发布。" action={<Button disabled={defaults.isPending} onClick={() => defaults.mutate()}>导入默认模板</Button>} /></Panel> : <Panel className="mt-4 overflow-hidden"><div className="overflow-x-auto"><table className="w-full min-w-[760px] text-left text-sm">
      <thead className="bg-muted/30 text-xs text-muted-foreground"><tr>{['类型', '内容', '状态', '操作'].map(title => <th key={title} className="px-4 py-3 font-medium">{title}</th>)}</tr></thead>
      <tbody>{pool.data.templates.map(t => <tr key={t.id} className="border-t border-border align-top">
        <td className="whitespace-nowrap px-4 py-4"><Badge>{t.probe_kind === 'fingerprint' ? '指纹' : 'Tokenizer'}</Badge></td>
        <td className="px-4 py-4"><p className="line-clamp-3 whitespace-pre-wrap break-all text-xs leading-5">{t.content}</p></td>
        <td className="whitespace-nowrap px-4 py-4">{t.retired_at ? <Badge>已停用</Badge> : inLatest.has(t.id) ? <Badge tone="success">已发布</Badge> : <Badge tone="warning">待发布</Badge>}</td>
        <td className="px-4 py-4">{!t.retired_at && <Button size="sm" variant="ghost" onClick={() => { retirement.reset(); setRetire(t) }}>停用</Button>}</td>
      </tr>)}</tbody>
    </table></div></Panel>}
    <FormDialog open={!!adding} onOpenChange={open => { if (!open && !create.isPending) setAdding(null) }} title="添加模板" description="指纹模板要求模型凭直觉写出一串整数，必须包含 {n}（数量）和 {r}（取值上限）。Tokenizer 模板是固定文本，用于比较不同厂商的 Token 计数。">
      {adding && <form className="grid gap-4" onSubmit={e => { e.preventDefault(); create.mutate(adding) }}>
        <Field label="类型">{id => <Choice id={id} name="probe_kind" value={adding.probe_kind} onChange={probe_kind => setAdding({ ...adding, probe_kind })} options={[{ value: 'fingerprint', label: '指纹' }, { value: 'token', label: 'Tokenizer' }]} />}</Field>
        <Field label="内容" hint={adding.probe_kind === 'fingerprint' ? '例如：凭第一反应写出 {n} 个 1 到 {r} 之间的整数……只输出逗号分隔的数字。' : '10–4000 个字符。请准备长度明显不同的多段文本。'}>{id => <textarea id={id} name="content" required className={textareaClass} value={adding.content} onChange={e => setAdding({ ...adding, content: e.target.value })} />}</Field>
        <ErrorBox error={create.error} />
        <div className="flex justify-end gap-2"><Button type="button" variant="outline" onClick={() => setAdding(null)}>取消</Button><Button type="submit" disabled={create.isPending}>{create.isPending ? '保存中…' : '保存'}</Button></div>
      </form>}
    </FormDialog>
    <Confirm open={!!retire} onOpenChange={open => { if (!open && !retirement.isPending) setRetire(null) }} title="停用模板" description="停用后需要发布新版本才会生效。已发布的版本和基准不受影响。" pending={retirement.isPending} error={retirement.error} onConfirm={() => { if (retire) retirement.mutate(retire.id) }} />
  </>
}
