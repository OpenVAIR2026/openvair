import { useEffect, useRef, useState } from 'react'
import { useMutation } from '@tanstack/react-query'
import { LoaderCircle, RefreshCw, X } from 'lucide-react'
import { api, ApiError, jsonBody, type Channel, type ModelCatalogEntry, type Protocol } from '../lib/api'
import { isConcreteModel } from '../lib/model-rules'
import { queryClient, refreshModels, useUser } from '../lib/queries'
import { Badge, Empty, ErrorBox } from './common'
import { Button } from './ui/button'
import { Checkbox } from './ui/checkbox'
import { Sheet, SheetClose, SheetContent, SheetDescription, SheetHeader, SheetTitle } from './ui/sheet'

type Scan = { id: string; name: string; state: 'waiting' | 'running' | 'done' | 'failed'; found: number; error?: string }
type Found = { pattern: string; protocols: Protocol[]; channels: { id: string; name: string }[]; fresh: boolean }
type AddResult = { added: number; skipped: number; failed: string[] }

/** Asks each enabled upstream channel for its models, a few at a time, and
 * lists the ones the catalog lacks as the answers come in. */
export function ModelDiscoverSheet({ onClose, onAdded }: { onClose: () => void; onAdded: (count: number) => void }) {
  const user = useUser()
  const catalogKey = ['model-catalog', user.account_id]
  const [scans, setScans] = useState<Scan[]>([])
  const [found, setFound] = useState<Found[]>([])
  const [selected, setSelected] = useState<string[]>([])
  const [error, setError] = useState<Error | null>(null)
  const [scanning, setScanning] = useState(false)
  // Shows only the models one channel offers; null shows all.
  const [filter, setFilter] = useState<string | null>(null)
  const run = useRef(0)
  const [progress, setProgress] = useState({ done: 0, total: 0 })

  async function scan() {
    const id = ++run.current
    const live = () => run.current === id
    setScanning(true); setError(null); setFound([]); setSelected([]); setScans([]); setFilter(null)
    try {
      const [channels, catalog] = await Promise.all([api<Channel[]>('/channels'), api<ModelCatalogEntry[]>('/model-catalog')])
      if (!live()) return
      queryClient.setQueryData(catalogKey, catalog)
      const known = new Set(catalog.map(model => model.pattern))
      const targets = channels.filter(channel => channel.kind === 'upstream' && channel.enabled)
      setScans(targets.map(channel => ({ id: channel.id, name: channel.name, state: 'waiting', found: 0 })))
      const update = (channelId: string, patch: Partial<Scan>) => setScans(current => current.map(item => item.id === channelId ? { ...item, ...patch } : item))
      const queue = [...targets]
      // Three channels at a time keeps slow upstreams from holding up the rest.
      await Promise.all(Array.from({ length: Math.min(3, queue.length) }, async () => {
        for (let channel = queue.shift(); channel && live(); channel = queue.shift()) {
          const current = channel
          update(current.id, { state: 'running' })
          try {
            const result = await api<{ models: string[] }>(`/channels/${current.id}/sync-models`, { method: 'POST' })
            if (!live()) return
            const before = new Set(current.discovered_models)
            const models = [...new Set(result.models)].filter(model => isConcreteModel(model) && !known.has(model))
            const protocol = current.protocol === 'any' ? null : current.protocol
            setFound(list => {
              const next = new Map(list.map(item => [item.pattern, item]))
              for (const model of models) {
                const item = next.get(model) ?? { pattern: model, protocols: [], channels: [], fresh: false }
                next.set(model, {
                  ...item,
                  protocols: protocol && !item.protocols.includes(protocol) ? [...item.protocols, protocol] : item.protocols,
                  channels: [...item.channels, { id: current.id, name: current.name }],
                  fresh: item.fresh || (!!current.models_synced_at && !before.has(model)),
                })
              }
              return [...next.values()]
            })
            update(current.id, { state: 'done', found: models.length })
          } catch (reason) {
            if (live()) update(current.id, { state: 'failed', error: reason instanceof Error ? reason.message : '获取失败' })
          }
        }
      }))
    } catch (reason) {
      if (live()) setError(reason instanceof Error ? reason : new Error('获取渠道失败'))
    } finally {
      if (live()) { setScanning(false); void queryClient.invalidateQueries({ queryKey: ['channels'] }) }
    }
  }
  useEffect(() => { void scan(); return () => { run.current++ } }, []) // eslint-disable-line react-hooks/exhaustive-deps

  const add = useMutation({
    mutationFn: async (models: Found[]): Promise<AddResult> => {
      const result: AddResult = { added: 0, skipped: 0, failed: [] }
      setProgress({ done: 0, total: models.length })
      for (let offset = 0; offset < models.length; offset += 4) {
        const batch = models.slice(offset, offset + 4)
        const results = await Promise.allSettled(batch.map(model => api<ModelCatalogEntry>('/model-catalog', jsonBody({ pattern: model.pattern, label: model.pattern, enabled: true, ...(model.protocols.length ? { protocols: model.protocols } : {}) }))))
        results.forEach((item, index) => {
          if (item.status === 'fulfilled') result.added++
          else if (item.reason instanceof ApiError && item.reason.status === 409) result.skipped++
          else result.failed.push(batch[index].pattern)
        })
        setProgress({ done: Math.min(offset + batch.length, models.length), total: models.length })
      }
      return result
    },
    onSuccess: (result, models) => {
      const failed = new Set(result.failed)
      setFound(list => list.filter(item => failed.has(item.pattern) || !models.some(model => model.pattern === item.pattern)))
      setSelected(result.failed)
      if (result.added) onAdded(result.added)
    },
    onSettled: refreshModels,
  })

  const sorted = [...found].sort((a, b) => Number(b.fresh) - Number(a.fresh) || a.pattern.localeCompare(b.pattern, 'en', { numeric: true }))
  const shown = filter ? sorted.filter(model => model.channels.some(item => item.id === filter)) : sorted
  const chosen = sorted.filter(model => selected.includes(model.pattern))
  const shownChosen = shown.filter(model => selected.includes(model.pattern)).length
  const selectShown = (checked: boolean) => {
    const ids = new Set(shown.map(model => model.pattern))
    setSelected(current => checked ? [...new Set([...current, ...ids])] : current.filter(pattern => !ids.has(pattern)))
  }
  const finished = scans.filter(item => item.state === 'done' || item.state === 'failed').length
  const busy = scanning || add.isPending
  const status = error ? '无法读取渠道' : !scans.length ? (scanning ? '正在读取渠道…' : '没有启用的上游渠道') : scanning ? `正在检查渠道 ${finished}/${scans.length}` : `已检查 ${scans.length} 个渠道`

  return <Sheet open onOpenChange={open => { if (!open && !add.isPending) onClose() }}>
    <SheetContent side="right" showCloseButton={false} className="w-full gap-0 p-0 sm:max-w-2xl">
      <SheetHeader className="shrink-0 gap-1 border-b border-border px-4 py-4 pr-16">
        <SheetTitle>发现新模型</SheetTitle>
        <SheetDescription className="text-xs leading-5">逐个读取已启用上游渠道的模型列表，列出目录里还没有的模型；勾选后加入目录。</SheetDescription>
        <SheetClose asChild><Button variant="ghost" size="icon-lg" className="absolute right-4 top-4" aria-label="关闭发现新模型" disabled={add.isPending}><X aria-hidden="true" /></Button></SheetClose>
      </SheetHeader>
      <div className="shrink-0 space-y-2 border-b border-border px-4 py-3">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <p role="status" className="flex items-center gap-2 text-sm tabular-nums">{scanning && <LoaderCircle aria-hidden="true" className="size-4 motion-safe:animate-spin" />}{status}</p>
          <Button variant="ghost" className="h-10" disabled={busy} onClick={() => void scan()}><RefreshCw aria-hidden="true" />重新检查</Button>
        </div>
        <ErrorBox error={error} retry={() => void scan()} />
        {!!scans.length && <ul className="flex flex-wrap gap-1.5" aria-label="渠道检查进度">
          {scans.map(item => {
            const label = `${item.name}${item.state === 'waiting' ? ' · 等待' : item.state === 'running' ? ' · 读取中' : item.state === 'failed' ? ' · 失败' : ` · ${item.found}`}`
            const active = filter === item.id
            return <li key={item.id} title={item.error}>
              {item.state === 'done' && item.found
                ? <button type="button" aria-pressed={active} onClick={() => setFilter(active ? null : item.id)} className={`rounded-md focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring ${active ? 'ring-2 ring-primary' : 'hover:opacity-80'}`}><Badge tone="success">{label}</Badge></button>
                : <Badge tone={item.state === 'failed' ? 'warning' : 'neutral'}>{label}</Badge>}
            </li>
          })}
        </ul>}
        {(scans.some(item => item.state === 'done' && item.found) || scans.some(item => item.state === 'failed')) && <p className="text-xs text-muted-foreground">{scans.some(item => item.state === 'done' && item.found) && '点击渠道标签只看它的模型，再点一次取消。'}{scans.some(item => item.state === 'failed') && '失败的渠道可将鼠标移到标签上查看原因。'}</p>}
      </div>
      <div className="flex shrink-0 flex-wrap items-center justify-between gap-x-3 border-b border-border px-4 py-2">
        <label className="flex min-h-10 items-center gap-2 text-sm">
          <Checkbox aria-label={filter ? '勾选当前渠道的新模型' : '勾选全部新模型'} disabled={add.isPending || !shown.length} checked={!!shown.length && shownChosen === shown.length ? true : shownChosen ? 'indeterminate' : false} onCheckedChange={checked => selectShown(checked === true)} />
          {filter ? '勾选当前渠道' : '勾选全部'}
        </label>
        <p className="flex items-center gap-2 text-xs tabular-nums text-muted-foreground">{filter ? `${scans.find(item => item.id === filter)?.name ?? ''}：${shown.length} / ${sorted.length} 个` : `目录中没有的模型 ${sorted.length} 个`}{filter && <Button variant="link" className="h-auto px-0 text-xs" onClick={() => setFilter(null)}>显示全部</Button>}</p>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain" role="region" aria-label="渠道新模型列表" tabIndex={0}>
        {shown.length ? <ul className="divide-y divide-border">
          {shown.map(model => {
            const checked = selected.includes(model.pattern)
            return <li key={model.pattern}>
              <label className={`flex min-h-16 cursor-pointer items-center gap-3 px-4 py-3 ${checked ? 'bg-primary/5' : 'hover:bg-accent'}`}>
                <Checkbox aria-label={`选择加入 ${model.pattern}`} disabled={add.isPending} checked={checked} onCheckedChange={value => setSelected(current => value === true ? [...new Set([...current, model.pattern])] : current.filter(pattern => pattern !== model.pattern))} />
                <span className="min-w-0 flex-1">
                  <code className="block break-all text-xs leading-5">{model.pattern}</code>
                  <span className="mt-1 block break-words text-xs text-muted-foreground">{model.channels.map(item => item.name).join('、')}{model.protocols.length ? ` · ${model.protocols.join(' / ')}` : ''}</span>
                </span>
                {model.fresh && <Badge tone="success">上次同步后新增</Badge>}
              </label>
            </li>
          })}
        </ul> : !scanning && !error && <Empty title="没有发现新模型" description="已检查渠道返回的模型都已在目录中。" />}
      </div>
      <div className="shrink-0 space-y-3 border-t border-border bg-background p-4">
        <ErrorBox error={add.error} />
        {add.isSuccess && <div role={add.data.failed.length ? 'alert' : 'status'} className={`text-sm ${add.data.failed.length ? 'text-destructive' : 'text-success'}`}>
          已加入 {add.data.added} 个模型。{add.data.skipped > 0 && ` ${add.data.skipped} 个已在目录中，已跳过。`}{add.data.failed.length > 0 && ` ${add.data.failed.length} 个添加失败，已保留勾选，请重试。`}
        </div>}
        <p className="text-xs text-muted-foreground">新模型默认待定价，协议取发现它的渠道的协议；加入后可设置价格或同步参考价。</p>
        <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
          <p className="text-sm tabular-nums">已选 {chosen.length} 个{chosen.length > shownChosen && <span className="ml-1 text-xs text-muted-foreground">（含其他渠道 {chosen.length - shownChosen} 个）</span>}</p>
          <div className="ml-auto flex gap-2">
            <Button variant="outline" className="h-10" disabled={add.isPending} onClick={onClose}>{add.isSuccess ? '完成' : '取消'}</Button>
            <Button className="h-10" disabled={add.isPending || !chosen.length} onClick={() => add.mutate(chosen)}>
              {add.isPending && <LoaderCircle aria-hidden="true" className="motion-safe:animate-spin" />}
              {add.isPending ? `加入中 ${progress.done}/${progress.total}` : `加入所选（${chosen.length}）`}
            </Button>
          </div>
        </div>
      </div>
    </SheetContent>
  </Sheet>
}
