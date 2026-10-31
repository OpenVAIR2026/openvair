import { useState } from 'react'
import { useMutation, useQuery } from '@tanstack/react-query'
import { ArrowRight, RefreshCw, X } from 'lucide-react'
import { api, jsonBody, type PriceRates, type PriceSyncChange } from '../lib/api'
import { queryClient, refreshModels, useUser } from '../lib/queries'
import { Empty, ErrorBox, Loading } from './common'
import { Button } from './ui/button'
import { Checkbox } from './ui/checkbox'
import { Sheet, SheetClose, SheetContent, SheetDescription, SheetHeader, SheetTitle } from './ui/sheet'

const rates: [keyof PriceRates, string][] = [['input_per_1m', '输入'], ['output_per_1m', '输出'], ['cached_read_per_1m', '缓存读取']]
const money = (value: number) => value.toLocaleString('en-US', { maximumSignificantDigits: 12 })

function PriceChange({ current, reference }: { current: number | undefined; reference: number }) {
  const changed = current !== reference
  return <div className="relative flex items-center justify-end gap-2 whitespace-nowrap font-mono tabular-nums">
    {changed && <>
      <span className="text-muted-foreground"><span className="sr-only">当前价格：{current == null ? '未定价' : ''}</span>{current == null ? <span aria-hidden="true">—</span> : money(current)}</span>
      <ArrowRight aria-hidden="true" className="size-3 shrink-0 text-muted-foreground" />
    </>}
    <span className={changed ? 'font-medium text-primary' : 'text-muted-foreground'}><span className="sr-only">{changed ? '同步后：' : '价格未变：'}</span>{money(reference)}</span>
  </div>
}

export function ModelPriceSyncSheet({ onClose, onSynced }: { onClose: () => void; onSynced: (count: number) => void }) {
  const user = useUser()
  const queryKey = ['model-price-sync-preview', user.account_id]
  const [selected, setSelected] = useState<string[]>([])
  const preview = useQuery({
    queryKey,
    queryFn: ({ signal }) => api<PriceSyncChange[]>('/platform/model-pricing/sync-preview', { signal }),
    staleTime: 0,
    gcTime: 0,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  })
  const sync = useMutation({
    mutationFn: (changes: PriceSyncChange[]) => api<{ synced: number }>('/platform/model-pricing/sync', jsonBody({ changes })),
    onSuccess: async (result, changes) => {
      const applied = new Set(changes.map(change => change.id))
      queryClient.setQueryData<PriceSyncChange[]>(queryKey, current => current?.filter(change => !applied.has(change.id)))
      setSelected([])
      onSynced(result.synced)
      await refreshModels()
    },
  })
  const changes = preview.data ?? []
  const chosen = changes.filter(change => selected.includes(change.id))
  const busy = preview.isFetching || sync.isPending
  const refresh = () => { setSelected([]); sync.reset(); void preview.refetch() }
  return <Sheet open onOpenChange={open => { if (!open && !sync.isPending) onClose() }}>
    <SheetContent side="right" showCloseButton={false} className="w-full gap-0 p-0 sm:max-w-4xl">
      <SheetHeader className="shrink-0 gap-1 border-b border-border px-4 py-4 pr-16">
        <SheetTitle>同步参考价</SheetTitle>
        <SheetDescription className="text-xs leading-5">当前价格 → OpenRouter 同名参考价 · 美元 / 百万 Token · 国产厂商只取官方自营价，分时段计价取高峰价</SheetDescription>
        <SheetClose asChild><Button variant="ghost" size="icon-lg" className="absolute right-4 top-4" aria-label="关闭价格同步" disabled={sync.isPending}><X aria-hidden="true" /></Button></SheetClose>
      </SheetHeader>
      <div className="flex shrink-0 flex-wrap items-center justify-between gap-2 border-b border-border px-4 py-2">
        <label className="flex min-h-10 items-center gap-2 text-sm"><Checkbox aria-label="选择全部差异模型" disabled={busy || !!preview.error || !changes.length} checked={!!changes.length && chosen.length === changes.length ? true : chosen.length ? 'indeterminate' : false} onCheckedChange={checked => setSelected(checked ? changes.map(change => change.id) : [])} />{preview.isFetching ? '正在比较价格…' : preview.error ? '暂时无法获取差异' : `${changes.length} 个模型有差异`}</label>
        <Button variant="ghost" className="h-10" disabled={busy} onClick={refresh}><RefreshCw aria-hidden="true" className={preview.isFetching ? 'motion-safe:animate-spin' : ''} />重新获取</Button>
      </div>
      <div className="relative min-h-0 flex-1 overflow-auto overscroll-contain" role="region" aria-label="模型价格差异" tabIndex={0}>
        {preview.error && <div className="p-4"><ErrorBox error={preview.error} retry={refresh} /></div>}
        {preview.isFetching ? <Loading /> : !preview.error && (changes.length ? <table aria-label="模型价格对比" className="w-full min-w-[720px] border-separate border-spacing-0 text-left text-xs">
          <thead className="sticky top-0 z-20"><tr className="text-muted-foreground">
            <th scope="col" className="sticky left-0 z-10 w-52 min-w-52 border-b border-border bg-secondary py-3 pl-10 pr-4 font-medium sm:w-64 sm:min-w-64">模型</th>
            {rates.map(([key, label]) => <th key={key} scope="col" className="min-w-36 whitespace-nowrap border-b border-border bg-secondary px-4 py-3 text-right font-medium">{label}</th>)}
          </tr></thead>
          <tbody>{changes.map(change => <tr key={change.id} aria-label={`${change.pattern} 价格对比`} className={`group ${selected.includes(change.id) ? 'bg-accent' : 'hover:bg-accent'}`}>
            <th scope="row" className={`sticky left-0 z-10 border-b border-border px-4 py-1 font-normal ${selected.includes(change.id) ? 'bg-accent' : 'bg-background group-hover:bg-accent'}`}>
              <label className="flex min-h-10 cursor-pointer items-center gap-2">
                <Checkbox aria-label={`选择同步 ${change.pattern}`} disabled={sync.isPending} checked={selected.includes(change.id)} onCheckedChange={checked => setSelected(current => checked ? [...current, change.id] : current.filter(id => id !== change.id))} />
                <span className="min-w-0 flex-1"><code className="block max-w-40 break-all leading-5 sm:max-w-56" title={change.label}>{change.pattern}</code>{!change.current && <span className="text-warning">待定价</span>}</span>
              </label>
            </th>
            {rates.map(([key]) => <td key={key} className="border-b border-border px-4 py-1"><PriceChange current={change.current?.[key]} reference={change.reference[key]} /></td>)}
          </tr>)}</tbody>
        </table> : <Empty title="没有价格差异" description="当前目录中有完整同名报价的模型均已一致。缺少报价的模型保持原价。" />)}
      </div>
      <div className="shrink-0 space-y-2 border-t border-border bg-background px-4 py-3">
        <ErrorBox error={sync.error} />
        {sync.isSuccess && <p role="status" className="text-sm text-success">已同步 {sync.data.synced} 个模型。</p>}
        <div className="flex flex-wrap items-center justify-between gap-3"><p className="text-xs text-muted-foreground">已选 {chosen.length} / {changes.length} 个</p><div className="flex gap-2">
          <Button variant="outline" className="h-10" disabled={busy || !!preview.error || !changes.length} onClick={() => sync.mutate(changes)}>同步所有（{changes.length}）</Button>
          <Button className="h-10" disabled={busy || !!preview.error || !chosen.length} onClick={() => sync.mutate(chosen)}>{sync.isPending ? '同步中…' : `同步所选（${chosen.length}）`}</Button>
        </div></div>
      </div>
    </SheetContent>
  </Sheet>
}
