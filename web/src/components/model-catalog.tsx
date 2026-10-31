import { useEffect, useState, type FormEvent } from 'react'
import { useMutation } from '@tanstack/react-query'
import { Pencil, Plus, Power, RefreshCw, Sparkles, Trash2 } from 'lucide-react'
import { api, jsonBody, type ModelCatalogEntry, type ModelPrice, type Protocol } from '../lib/api'
import { refreshModels, useModelCatalog } from '../lib/queries'
import { isConcreteModel } from '../lib/model-rules'
import { Button } from './ui/button'
import { Input } from './ui/input'
import { Checkbox } from './ui/checkbox'
import { Badge, Choice, Confirm, DataTable, Empty, ErrorBox, Field, FormSheet, Loading } from './common'
import { UnpricedModelsPanel } from './unpriced-models'
import { ModelPriceSyncSheet } from './model-price-sync'
import { ModelAddSheet } from './model-add'
import { ModelDiscoverSheet } from './model-discover'

const protocols: Protocol[] = ['openai', 'anthropic', 'gemini']
type Editor = { id?: string; pattern: string; label: string; protocols: Protocol[]; enabled: boolean; priced: boolean; input: string; output: string; cached: string; onlyUnpriced?: boolean }
const empty: Editor = { pattern: '', label: '', protocols, enabled: true, priced: false, input: '', output: '', cached: '' }
const toEditor = (entry: ModelCatalogEntry): Editor => ({ id: entry.id, pattern: entry.pattern, label: entry.label, protocols: entry.protocols, enabled: entry.enabled, priced: entry.input_per_1m != null, input: entry.input_per_1m?.toString() ?? '', output: entry.output_per_1m?.toString() ?? '', cached: entry.cached_read_per_1m?.toString() ?? '' })
const money = (value: number | null) => value == null ? '—' : value.toLocaleString('en-US', { maximumFractionDigits: 8 })

/** One view of the catalog; the page's tabs pick which. */
export function ModelCatalogPanel({ view, onView }: { view: 'models' | 'pending'; onView: (view: 'models') => void }) {
  const catalog = useModelCatalog()
  const [editor, setEditor] = useState<Editor | null>(null)
  const [removeTarget, setRemoveTarget] = useState<ModelCatalogEntry | null>(null)
  const [selected, setSelected] = useState<string[]>([])
  useEffect(() => setSelected([]), [view])
  const [batch, setBatch] = useState(false)
  const [message, setMessage] = useState('')
  const [syncOpen, setSyncOpen] = useState(false)
  const [addOpen, setAddOpen] = useState(false)
  const [discoverOpen, setDiscoverOpen] = useState(false)
  // The API hides legacy prefix templates; filter as well so none can be edited here.
  const rows = (catalog.data ?? []).filter(item => isConcreteModel(item.pattern))
  const save = useMutation({ mutationFn: async (form: Editor) => {
    if ((!batch && !isConcreteModel(form.pattern.trim())) || !form.label.trim()) throw new Error('请填写完整的具体模型 ID 和显示名称；前缀规则请在渠道或 API Key 中填写')
    if (!form.protocols.length) throw new Error('至少选择一种调用协议')
    let price: ModelPrice | undefined
    if (form.priced) {
      const numbers = [form.input, form.output, form.cached]
      if (numbers.some(value => !value.trim() || !Number.isFinite(Number(value)) || Number(value) < 0)) throw new Error('请填写完整非负价格，免费项目请明确填 0')
      price = { pattern: form.pattern.trim(), label: form.label.trim(), input_per_1m: Number(form.input), output_per_1m: Number(form.output), cached_read_per_1m: Number(form.cached), synced_from: null }
    }
    const payload = { pattern: form.pattern.trim(), label: form.label.trim(), protocols: form.protocols, enabled: form.enabled, price, clear_price: !price, ...(form.onlyUnpriced ? { only_unpriced: true } : {}) }
    if (batch) {
      const targets = (catalog.data ?? []).filter(item => selected.includes(item.id) && isConcreteModel(item.pattern))
      const results = await Promise.allSettled(targets.map(item => api(`/model-catalog/${item.id}`, jsonBody({ pattern: item.pattern, label: item.label, price: price ? { ...price, pattern: item.pattern, label: item.label } : undefined, clear_price: !price }, 'PATCH'))))
      const failed = results.filter(result => result.status === 'rejected').length
      if (failed) throw new Error(`${targets.length - failed} 个模型已保存，${failed} 个失败；请刷新后重试失败项。`)
    } else await api(form.id ? `/model-catalog/${form.id}` : '/model-catalog', jsonBody(payload, form.id ? 'PATCH' : 'POST'))
  }, onSuccess: () => { setEditor(null); setSelected([]); setBatch(false); setMessage('已保存。') }, onSettled: refreshModels })
  const remove = useMutation({ mutationFn: (id: string) => api(`/model-catalog/${id}`, { method: 'DELETE' }), onSuccess: () => { setRemoveTarget(null); setMessage('已删除。') }, onSettled: refreshModels })
  const toggle = useMutation({ mutationFn: (item: ModelCatalogEntry) => api(`/model-catalog/${item.id}`, jsonBody({ pattern: item.pattern, label: item.label, enabled: !item.enabled }, 'PATCH')), onSettled: refreshModels })
  const addPrice = useMutation({ mutationFn: async (price: ModelPrice) => {
    const current = await api<ModelCatalogEntry[]>('/model-catalog')
    const entry = current.find(item => item.pattern === price.pattern)
    const payload = { pattern: price.pattern, label: entry?.label ?? price.label, price, only_unpriced: true }
    return api(entry ? `/model-catalog/${entry.id}` : '/model-catalog', jsonBody(payload, entry ? 'PATCH' : 'POST'))
  }, onSettled: refreshModels })
  const syncOne = useMutation({ mutationFn: (pattern: string) => api('/platform/model-pricing/sync-one', jsonBody({ pattern })), onMutate: () => setMessage(''), onSettled: refreshModels })
  function openEditor(entry?: ModelCatalogEntry, model?: string) { save.reset(); setBatch(false); setEditor(entry ? toEditor(entry) : { ...empty, pattern: model ?? '', label: model ?? '', priced: !!model }) }
  function manual(model: string) {
    onView('models')
    const found = catalog.data?.find(item => item.pattern === model && isConcreteModel(model))
    // A pending model is priced in place; only_unpriced rejects a price set meanwhile.
    if (found && found.input_per_1m == null) { save.reset(); setBatch(false); setEditor({ ...toEditor(found), priced: true, onlyUnpriced: true }) } else openEditor(found, model)
  }
  function submit(event: FormEvent) { event.preventDefault(); if (editor) save.mutate(editor) }
  return <div className="space-y-5">
    {view !== 'pending' && <div className="flex flex-wrap items-center justify-between gap-3">
      {view === 'models' && <p className="text-xs text-muted-foreground">价格单位：美元 / 百万 Token</p>}
      <div className="ml-auto flex flex-wrap gap-2">{view === 'models' && <Button variant="outline" disabled={syncOne.isPending} onClick={() => { syncOne.reset(); setSyncOpen(true) }} title="预览同名模型的价格差异，再选择同步"><RefreshCw aria-hidden="true" />同步参考价</Button>}{view === 'models' && <Button variant="outline" onClick={() => { setMessage(''); setDiscoverOpen(true) }} title="读取各渠道的模型列表，勾选目录中没有的模型加入"><Sparkles aria-hidden="true" />发现新模型</Button>}<Button onClick={() => { setMessage(''); setAddOpen(true) }}><Plus aria-hidden="true" />添加模型</Button></div>
    </div>}
    {message && <p role="status" className="text-sm text-success">{message}</p>}
    <ErrorBox error={catalog.error ?? toggle.error} retry={() => catalog.refetch()} />
    {view === 'pending' ? <>
      <UnpricedModelsPanel disabled={addPrice.isPending} adding={addPrice.isPending ? addPrice.variables?.pattern ?? null : null} error={addPrice.error} added={addPrice.isSuccess ? addPrice.variables?.pattern ?? null : null} onAdd={price => addPrice.mutate(price)} onManual={manual} />
    </> : <>
      {view === 'models' && !!selected.length && <div className="flex items-center gap-3"><span className="text-sm">已选 {selected.length} 个具体模型</span><Button variant="outline" onClick={() => { save.reset(); setBatch(true); setEditor({ ...empty, pattern: 'batch', label: 'batch', priced: true }) }}>批量设置独立价格</Button><Button variant="ghost" onClick={() => setSelected([])}>取消选择</Button></div>}
      {catalog.isPending ? <Loading /> : !catalog.error && <DataTable key={view} data={rows} columns={[
        ...(view === 'models' ? [{ id: 'select', header: '选择', cell: ({ row }: { row: { original: ModelCatalogEntry } }) => <label className="flex min-h-10 min-w-10 items-center"><Checkbox aria-label={`选择 ${row.original.pattern}`} checked={selected.includes(row.original.id)} onCheckedChange={checked => setSelected(current => checked ? [...current, row.original.id] : current.filter(id => id !== row.original.id))} /></label> }] : []),
        { accessorKey: 'pattern', header: '模型 ID', cell: ({ row }) => <div className="min-w-40"><code className="break-all text-xs">{row.original.pattern}</code><p className="mt-1 text-xs text-muted-foreground">{row.original.label}</p></div> },
        { id: 'protocols', header: '调用协议', cell: ({ row }) => <span className="text-xs text-muted-foreground">{row.original.protocols?.join(' / ')}</span> },
        ...(view === 'models' ? [{ id: 'price', header: '输入 / 输出 / 缓存读', cell: ({ row }: { row: { original: ModelCatalogEntry } }) => {
          const item = row.original
          const active = syncOne.variables === item.pattern
          const syncing = active && syncOne.isPending
          return <div className="space-y-1">
            <div className="flex items-center gap-1">
              <span className="whitespace-nowrap font-mono text-xs tabular-nums">{money(item.input_per_1m)} / {money(item.output_per_1m)} / {money(item.cached_read_per_1m)}</span>
              <Button variant="ghost" size="icon-lg" disabled={syncOne.isPending} aria-label={`同步价格 ${item.pattern}`} aria-busy={syncing} title="从 OpenRouter 同步同名模型的参考价" onClick={() => syncOne.mutate(item.pattern)}><RefreshCw aria-hidden="true" className={syncing ? 'motion-safe:animate-spin' : ''} /></Button>
            </div>
            <div><Badge tone={item.input_per_1m == null ? 'warning' : 'success'}>{item.input_per_1m == null ? '待定价' : '独立定价'}</Badge></div>
            {active && <div className="max-w-64 text-xs leading-5" aria-live="polite">{syncing ? <span className="text-muted-foreground">正在获取参考价…</span> : syncOne.isError ? <span role="alert" className="text-destructive">{syncOne.error.message}</span> : syncOne.isSuccess ? <span className="text-success">已同步 OpenRouter 参考价</span> : null}</div>}
          </div>
        } }] : []),
        { id: 'enabled', header: '目录状态', cell: ({ row }) => <Badge tone={row.original.enabled ? 'success' : 'neutral'}>{row.original.enabled ? '可选' : '已隐藏'}</Badge> },
        { id: 'actions', header: '操作', cell: ({ row }) => <div className="flex gap-1"><Button variant="ghost" size="icon" aria-label={`编辑 ${row.original.pattern}`} onClick={() => openEditor(row.original)}><Pencil /></Button><Button variant="ghost" size="icon" disabled={toggle.isPending} aria-label={`${row.original.enabled ? '隐藏' : '显示'} ${row.original.pattern}`} onClick={() => toggle.mutate(row.original)}><Power /></Button><Button variant="ghost" size="icon" aria-label={`删除 ${row.original.pattern}`} onClick={() => setRemoveTarget(row.original)}><Trash2 className="text-destructive" /></Button></div> },
      ]} empty={<Empty title="暂无模型" description="添加具体模型与价格，或切换到「待定价」标签。" />} />}
    </>}
    <FormSheet open={!!editor} onOpenChange={open => { if (!open && !save.isPending) setEditor(null) }} title={batch ? `为 ${selected.length} 个模型设置独立价格` : editor?.id ? '编辑模型' : '添加模型'}>
      {editor && <form className="grid gap-4" onSubmit={submit}><fieldset disabled={save.isPending} className="grid gap-4">
        {!batch && <>
          <Field label="模型 ID" help={editor.id ? '模型 ID 区分大小写，创建后不可修改。': '填写完整模型 ID，例如 gpt-4.1，区分大小写。'}>{id => <Input id={id} autoComplete="off" spellCheck={false} value={editor.pattern} disabled={!!editor.id} onChange={event => setEditor({ ...editor, pattern: event.target.value })} required />}</Field>
          <Field label="显示名称">{id => <Input id={id} value={editor.label} onChange={event => setEditor({ ...editor, label: event.target.value })} required />}</Field>
          <div><p className="mb-2 text-sm font-medium">调用协议</p><div className="flex flex-wrap gap-4">{protocols.map(protocol => <label key={protocol} className="flex min-h-10 items-center gap-2 text-sm"><Checkbox checked={editor.protocols.includes(protocol)} onCheckedChange={checked => setEditor({ ...editor, protocols: checked ? [...editor.protocols, protocol] : editor.protocols.filter(p => p !== protocol) })} />{protocol}</label>)}</div></div>
        </>}
        <>
          <Field label="定价状态" help="未定价模型可以正常调用，但这些请求不计入费用和费用限额。">{id => <Choice id={id} value={editor.priced ? 'priced' : 'unpriced'} onChange={value => setEditor({ ...editor, priced: value === 'priced' })} options={[{ value: 'unpriced', label: '待定价' }, { value: 'priced', label: '独立定价' }]} />}</Field>
          {editor.priced && <div className="grid gap-4 sm:grid-cols-3">{[['input', '输入 ($/1M)'], ['output', '输出 ($/1M)'], ['cached', '缓存读 ($/1M)']].map(([key, label]) => <Field key={key} label={label}>{id => <Input id={id} type="number" min="0" step="any" value={editor[key as 'input' | 'output' | 'cached']} onChange={event => setEditor({ ...editor, [key]: event.target.value })} required />}</Field>)}</div>}
        </>
      </fieldset><ErrorBox error={save.error} /><div className="flex justify-end gap-2 border-t border-border pt-4"><Button type="button" variant="outline" disabled={save.isPending} onClick={() => setEditor(null)}>取消</Button><Button type="submit" disabled={save.isPending}>{save.isPending ? '保存中…' : '保存模型'}</Button></div></form>}
    </FormSheet>
    {addOpen && <ModelAddSheet onClose={() => setAddOpen(false)} onManual={() => { setAddOpen(false); openEditor() }} onAdded={count => setMessage(`已加入 ${count} 个模型。`)} />}
    {discoverOpen && <ModelDiscoverSheet onClose={() => setDiscoverOpen(false)} onAdded={count => setMessage(`已加入 ${count} 个模型。`)} />}
    {syncOpen && <ModelPriceSyncSheet onClose={() => setSyncOpen(false)} onSynced={count => setMessage(`已同步 ${count} 个模型。`)} />}
    <Confirm open={!!removeTarget} onOpenChange={open => { if (!open) setRemoveTarget(null) }} title="从平台目录移除？" description={`移除 ${removeTarget?.pattern ?? ''} 及其价格。`} pending={remove.isPending} error={remove.error} onConfirm={() => { if (removeTarget) remove.mutate(removeTarget.id) }} />
  </div>
}
