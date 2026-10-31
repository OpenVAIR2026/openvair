import { useMemo, useRef, useState } from 'react'
import { useMutation } from '@tanstack/react-query'
import { LoaderCircle, Search, X } from 'lucide-react'
import { api, ApiError, jsonBody, type ModelCatalogEntry } from '../lib/api'
import { isConcreteModel } from '../lib/model-rules'
import { queryClient, refreshModels, useModelCatalog, useModelPricingCatalog, useUser } from '../lib/queries'
import { Badge, Empty, ErrorBox, Loading } from './common'
import { Button } from './ui/button'
import { Checkbox } from './ui/checkbox'
import { Input } from './ui/input'
import { Sheet, SheetClose, SheetContent, SheetDescription, SheetHeader, SheetTitle } from './ui/sheet'

type Candidate = { pattern: string; label: string }
type AddResult = { added: ModelCatalogEntry[]; skipped: number; failed: Candidate[] }
// Search tolerates case, spacing and separators; model identity stays exact.
const searchText = (value: string) => value.toLocaleLowerCase().replace(/[\s\-_.:/]+/g, '')

export function ModelAddSheet({ onClose, onManual, onAdded }: { onClose: () => void; onManual: () => void; onAdded: (count: number) => void }) {
  const user = useUser()
  const catalog = useModelCatalog()
  const reference = useModelPricingCatalog()
  const [search, setSearch] = useState('')
  const [selected, setSelected] = useState<string[]>([])
  const [progress, setProgress] = useState({ done: 0, total: 0 })
  const searchInput = useRef<HTMLInputElement>(null)
  const catalogKey = ['model-catalog', user.account_id]
  const existing = new Set(catalog.data?.map(model => model.pattern))
  const candidates = useMemo(() => {
    const models = new Map<string, Candidate>()
    for (const model of [...(reference.data ?? []), ...(catalog.data ?? [])]) {
      if (isConcreteModel(model.pattern)) models.set(model.pattern, { pattern: model.pattern, label: model.label })
    }
    return [...models.values()].sort((a, b) => a.pattern.localeCompare(b.pattern, 'en', { numeric: true }))
  }, [catalog.data, reference.data])
  const terms = search.trim().split(/\s+/).map(searchText).filter(Boolean)
  const visible = candidates.filter(model => terms.every(term => searchText(model.pattern).includes(term) || searchText(model.label).includes(term)))
  const available = visible.filter(model => !existing.has(model.pattern))
  const selectedSet = new Set(selected)
  const chosen = candidates.filter(model => selectedSet.has(model.pattern) && !existing.has(model.pattern))
  const visibleChosen = available.filter(model => selectedSet.has(model.pattern)).length
  const hiddenChosen = chosen.length - visibleChosen
  const add = useMutation({
    mutationFn: async (models: Candidate[]): Promise<AddResult> => {
      setProgress({ done: 0, total: models.length })
      // Recheck membership before writing; POST also rejects concurrent duplicates.
      const current = await api<ModelCatalogEntry[]>('/model-catalog')
      queryClient.setQueryData(catalogKey, current)
      const existingNow = new Set(current.map(model => model.pattern))
      const targets = models.filter(model => !existingNow.has(model.pattern))
      const result: AddResult = { added: [], skipped: models.length - targets.length, failed: [] }
      setProgress({ done: 0, total: targets.length })
      // Bound concurrency even when the user selects the entire reference catalog.
      for (let offset = 0; offset < targets.length; offset += 4) {
        const batch = targets.slice(offset, offset + 4)
        const results = await Promise.allSettled(batch.map(model => api<ModelCatalogEntry>('/model-catalog', jsonBody({ pattern: model.pattern, label: model.label, enabled: true }))))
        results.forEach((item, index) => {
          if (item.status === 'fulfilled') result.added.push(item.value)
          else if (item.reason instanceof ApiError && item.reason.status === 409) result.skipped++
          else result.failed.push(batch[index])
        })
        setProgress({ done: Math.min(offset + batch.length, targets.length), total: targets.length })
      }
      return result
    },
    onSuccess: result => {
      queryClient.setQueryData<ModelCatalogEntry[]>(catalogKey, current => {
        const models = new Map(current?.map(model => [model.pattern, model]))
        result.added.forEach(model => models.set(model.pattern, model))
        return [...models.values()]
      })
      setSelected(result.failed.map(model => model.pattern))
      if (result.added.length) onAdded(result.added.length)
    },
    onSettled: refreshModels,
  })
  const loading = catalog.isPending || reference.isPending
  const loadError = catalog.error ?? reference.error
  const busy = add.isPending || catalog.isFetching || reference.isFetching
  const retry = () => { void catalog.refetch(); void reference.refetch() }
  const selectAll = (checked: boolean) => {
    const visibleIds = new Set(available.map(model => model.pattern))
    setSelected(current => checked ? [...new Set([...current, ...visibleIds])] : current.filter(pattern => !visibleIds.has(pattern)))
  }

  return <Sheet open onOpenChange={open => { if (!open && !add.isPending) onClose() }}>
    <SheetContent side="right" showCloseButton={false} className="w-full gap-0 p-0 sm:max-w-2xl" onOpenAutoFocus={event => { event.preventDefault(); searchInput.current?.focus() }}>
      <SheetHeader className="shrink-0 gap-1 border-b border-border px-4 py-4 pr-16">
        <SheetTitle>添加模型</SheetTitle>
        <SheetDescription className="text-xs leading-5">搜索模型并批量加入目录，已加入的模型不可重复选择。</SheetDescription>
        <SheetClose asChild><Button variant="ghost" size="icon-lg" className="absolute right-4 top-4" aria-label="关闭添加模型" disabled={add.isPending}><X aria-hidden="true" /></Button></SheetClose>
      </SheetHeader>
      <div className="shrink-0 space-y-2 border-b border-border p-4">
        <label htmlFor="model-add-search" className="block text-sm font-medium">搜索模型</label>
        <div className="relative">
          <Search aria-hidden="true" className="pointer-events-none absolute left-3 top-3 size-4 text-muted-foreground" />
          <Input ref={searchInput} id="model-add-search" name="model-search" type="search" autoComplete="off" spellCheck={false} placeholder="输入模型 ID 或名称，如 gpt、Claude Sonnet" className="h-10 pl-9" value={search} disabled={add.isPending} onChange={event => setSearch(event.target.value)} />
        </div>
        <div className="flex flex-wrap items-center justify-between gap-x-3 text-xs text-muted-foreground">
          <span>候选来自 OpenRouter 与现有目录</span>
          <Button variant="link" className="h-10 px-0 text-xs" disabled={add.isPending} onClick={onManual}>手动添加</Button>
        </div>
      </div>
      <div className="flex shrink-0 flex-wrap items-center justify-between gap-x-3 border-b border-border px-4 py-2">
        <label className="flex min-h-10 items-center gap-2 text-sm">
          <Checkbox aria-label="勾选全部未加入模型" disabled={busy || !!loadError || !available.length} checked={!!available.length && visibleChosen === available.length ? true : visibleChosen ? 'indeterminate' : false} onCheckedChange={checked => selectAll(checked === true)} />
          {search.trim() ? '勾选当前结果未加入' : '勾选全部未加入'}
        </label>
        <p role="status" className="text-xs tabular-nums text-muted-foreground">{loading ? '正在获取模型…' : loadError ? '模型列表暂不可用' : `${search.trim() ? '匹配' : '共'} ${visible.length} 个 · 未加入 ${available.length} 个`}</p>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain" role="region" aria-label="可添加模型列表" tabIndex={0}>
        {loadError ? <div className="p-4"><ErrorBox error={new Error('模型列表加载失败，请重试。')} retry={retry} /></div> : loading ? <Loading /> : visible.length ? <ul className="divide-y divide-border">
          {visible.map(model => {
            const added = existing.has(model.pattern)
            const checked = !added && selectedSet.has(model.pattern)
            return <li key={model.pattern}>
              <label className={`flex min-h-16 items-center gap-3 px-4 py-3 ${added ? 'bg-muted/20' : checked ? 'cursor-pointer bg-primary/5' : 'cursor-pointer hover:bg-accent'}`}>
                <Checkbox aria-label={`${added ? '已加入' : '选择添加'} ${model.pattern}`} disabled={added || busy || !!loadError} checked={added || checked} onCheckedChange={value => setSelected(current => value === true ? [...new Set([...current, model.pattern])] : current.filter(pattern => pattern !== model.pattern))} />
                <span className="min-w-0 flex-1">
                  <code className={`block break-all text-xs leading-5 ${added ? 'text-muted-foreground' : 'text-foreground'}`}>{model.pattern}</code>
                  {model.label !== model.pattern && <span className="mt-1 block break-words text-xs text-muted-foreground">{model.label}</span>}
                </span>
                <Badge tone={added ? 'success' : 'neutral'}>{added ? '已加入' : '未加入'}</Badge>
              </label>
            </li>
          })}
        </ul> : <Empty title={search.trim() ? '未找到匹配模型' : '暂无候选模型'} description="尝试其他关键词，或手动添加完整模型 ID。" action={search.trim() && <Button variant="outline" onClick={() => setSearch('')}>清空搜索</Button>} />}
      </div>
      <div className="shrink-0 space-y-3 border-t border-border bg-background p-4">
        <ErrorBox error={add.error} />
        {add.isSuccess && <div role={add.data.failed.length ? 'alert' : 'status'} className={`text-sm ${add.data.failed.length ? 'text-destructive' : 'text-success'}`}>
          已加入 {add.data.added.length} 个模型。{add.data.skipped > 0 && ` ${add.data.skipped} 个模型已在目录中，已跳过。`}{add.data.failed.length > 0 && ` ${add.data.failed.length} 个添加失败，已保留勾选，请重试。`}
        </div>}
        <p className="text-xs text-muted-foreground">新模型默认待定价，可在加入后设置价格或同步参考价。</p>
        <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
          <div className="flex flex-wrap items-center gap-x-3">
            <p className="text-sm tabular-nums">已选 {chosen.length} 个{hiddenChosen > 0 && <span className="ml-1 text-xs text-muted-foreground">（含其他搜索结果 {hiddenChosen} 个）</span>}</p>
            {!!chosen.length && <Button variant="ghost" className="h-10 px-2 text-xs" disabled={add.isPending} onClick={() => setSelected([])}>清空选择</Button>}
          </div>
          <div className="ml-auto flex gap-2">
            <Button variant="outline" className="h-10" disabled={add.isPending} onClick={onClose}>{add.isSuccess ? '完成' : '取消'}</Button>
            <Button className="h-10" disabled={busy || !!loadError || !chosen.length} onClick={() => add.mutate(chosen)}>
              {add.isPending && <LoaderCircle aria-hidden="true" className="motion-safe:animate-spin" />}
              {add.isPending ? `添加中 ${progress.done}/${progress.total}` : `添加所选（${chosen.length}）`}
            </Button>
          </div>
        </div>
      </div>
    </SheetContent>
  </Sheet>
}
