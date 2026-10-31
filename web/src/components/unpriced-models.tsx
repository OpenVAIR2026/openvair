import { LoaderCircle, Plus, RefreshCw } from 'lucide-react'
import type { ColumnDef } from '@tanstack/react-table'
import { formatDate, formatShortDate, type ModelPrice, type UnpricedModel } from '../lib/api'
import { useModelPricingCatalog, useUnpricedModels } from '../lib/queries'
import { Badge, DataTable, Empty, ErrorBox, HelpTip, Loading } from './common'
import { Button } from './ui/button'

interface Props {
  disabled: boolean
  adding: string | null
  error: Error | null
  added: string | null
  onAdd: (price: ModelPrice) => void
  onManual: (model: string) => void
}

export function UnpricedModelsPanel({ disabled, adding, error, added, onAdd, onManual }: Props) {
  const unpriced = useUnpricedModels()
  const catalog = useModelPricingCatalog()
  // Only identical IDs have reliable reference prices. Similar names can be different models.
  const referencePrices = new Map(catalog.data?.map(price => [price.pattern, price]))
  const columns: ColumnDef<UnpricedModel & { id: string }>[] = [
    { accessorKey: 'model', header: '模型', cell: ({ row }) => <code className="block min-w-48 max-w-72 whitespace-normal break-all text-xs">{row.original.model}</code> },
    { accessorKey: 'requests', header: '调用次数', cell: ({ row }) => <span className="tabular-nums">{row.original.requests.toLocaleString()}</span> },
    { accessorKey: 'last_seen_at', header: '最近调用', cell: ({ row }) => <time dateTime={row.original.last_seen_at} title={formatDate(row.original.last_seen_at)} className="text-xs text-muted-foreground">{formatShortDate(row.original.last_seen_at)}</time> },
    { id: 'price', header: '参考价 · $/百万 Token', cell: ({ row }) => {
      const price = referencePrices.get(row.original.model)
      return price
        ? <div className="text-xs leading-5"><div className="tabular-nums">输入 {price.input_per_1m} · 输出 {price.output_per_1m}</div><div className="text-muted-foreground">缓存读 {price.cached_read_per_1m} · OpenRouter</div></div>
        : <span className="text-xs text-muted-foreground">{catalog.isPending ? '正在获取参考价…' : catalog.error ? '参考价暂不可用' : '暂无同名报价'}</span>
    } },
    { id: 'actions', header: '操作', cell: ({ row }) => {
      const model = row.original.model
      const price = referencePrices.get(model)
      return price
        ? <Button variant="outline" className="h-10" disabled={disabled} aria-label={`采用参考价 ${model}`} onClick={() => onAdd({ ...price, pattern: model })}>
            {adding === model ? <LoaderCircle aria-hidden="true" className="animate-spin" /> : <Plus aria-hidden="true" />}
            {adding === model ? '保存中…' : '采用参考价'}
          </Button>
        : <Button variant="outline" className="h-10" disabled={disabled} aria-label={`填写价格 ${model}`} onClick={() => onManual(model)}>填写价格</Button>
    } },
  ]

  return <section aria-labelledby="unpriced-models-title" className="mb-6 max-w-[960px] space-y-3">
    <div className="flex flex-wrap items-start justify-between gap-3">
      <div>
        <div className="flex items-center gap-2"><h2 id="unpriced-models-title" className="font-medium">待定价模型</h2><HelpTip label="待定价模型">全平台近 30 天调用过、尚未定价的上游模型，按调用次数排序，包含成功与失败请求。</HelpTip>{unpriced.data && <Badge tone={unpriced.data.models.length ? 'warning' : 'neutral'}>{unpriced.data.models.length} 个</Badge>}</div>
      </div>
      <Button variant="outline" className="h-10" disabled={unpriced.isFetching} aria-label="刷新待定价模型" onClick={() => { void unpriced.refetch() }}><RefreshCw aria-hidden="true" className={unpriced.isFetching ? 'animate-spin' : ''} />刷新</Button>
    </div>
    {added && <p role="status" className="break-all text-sm text-success">已为 {added} 采用参考价。</p>}
    <ErrorBox error={error} />
    {catalog.error && <ErrorBox error={new Error('参考价获取失败，仍可手动填写价格。')} retry={() => { void catalog.refetch() }} />}
    <ErrorBox error={unpriced.error} retry={() => { void unpriced.refetch() }} />
    {unpriced.isPending ? <Loading /> : unpriced.data && <DataTable
      data={unpriced.data.models.map(model => ({ ...model, id: model.model }))}
      columns={columns}
      empty={<Empty title="暂无待定价模型" description="近 30 天有调用的模型均已精确定价，或尚无调用记录。" />}
    />}
  </section>
}
