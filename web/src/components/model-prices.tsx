import { PRODUCT } from '../lib/edition'
import { useMemo, useState } from 'react'
import { Search } from 'lucide-react'
import { useModelCatalog, useResource } from '../lib/queries'
import { Empty, ErrorBox, FormSheet } from './common'
import { Checkbox } from './ui/checkbox'
import { Input } from './ui/input'

const rate = (value: number | null) => value == null ? null : `$${value.toFixed(value < 1 ? 3 : 2).replace(/0+$/, '').replace(/\.$/, '')}`

/** Read-only catalog prices: the gateway sells no tokens, these only estimate cost. */
export function ModelPricesSheet({ open, initialSearch = '', onOpenChange }: { open: boolean; initialSearch?: string; onOpenChange: (open: boolean) => void }) {
  const catalog = useModelCatalog()
  const available = useResource<Record<string, string>>('available-models')
  const [search, setSearch] = useState(initialSearch)
  const [mine, setMine] = useState(!initialSearch)
  const routable = useMemo(() => new Set(Object.keys(available.data ?? {})), [available.data])
  // With no routable model yet, "mine" would hide everything; show the whole catalog instead.
  const onlyMine = mine && routable.size > 0
  const rows = useMemo(() => {
    const term = search.trim().toLowerCase()
    return (catalog.data ?? []).filter(entry => (!onlyMine || routable.has(entry.pattern)) && (!term || entry.pattern.toLowerCase().includes(term) || entry.label.toLowerCase().includes(term)))
  }, [catalog.data, onlyMine, routable, search])
  return <FormSheet open={open} onOpenChange={onOpenChange} title="模型单价" description={`${PRODUCT} 不出售 Token，实际费用以上游厂商的帐单为准。这里的单价只用来估算使用分析里的费用和 Key 的费用上限。`}>
    <div className="grid gap-4">
      <div className="flex flex-wrap items-center gap-3">
        <div className="relative min-w-0 flex-1"><Search aria-hidden="true" className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" /><Input aria-label="搜索模型" placeholder="搜索模型…" className="pl-9" value={search} onChange={event => setSearch(event.target.value)} /></div>
        <label className="flex items-center gap-2 text-sm"><Checkbox checked={mine} onCheckedChange={value => setMine(value === true)} disabled={!routable.size} />只看本帐号可用</label>
      </div>
      <ErrorBox error={catalog.error ?? available.error} retry={() => { void catalog.refetch(); void available.refetch() }} />
      {catalog.isPending ? <div className="h-40 animate-pulse rounded-lg bg-muted" /> : rows.length ? <div className="overflow-x-auto rounded-lg border border-border">
        <table className="w-full text-sm">
          <thead className="bg-muted/40 text-xs text-muted-foreground"><tr><th className="px-3 py-2 text-left font-medium">模型</th><th className="px-3 py-2 text-right font-medium">输入</th><th className="px-3 py-2 text-right font-medium">输出</th><th className="px-3 py-2 text-right font-medium">缓存读</th></tr></thead>
          <tbody>{rows.map(entry => <tr key={entry.id} className="border-t border-border">
            <td className="px-3 py-2"><div className="font-mono text-[13px] break-all">{entry.pattern}</div>{!onlyMine && routable.has(entry.pattern) && <div className="text-xs text-success">本帐号可用</div>}</td>
            {entry.input_per_1m == null ? <td colSpan={3} className="px-3 py-2 text-right text-xs text-muted-foreground">待定价</td> : [entry.input_per_1m, entry.output_per_1m, entry.cached_read_per_1m].map((value, index) => <td key={index} className="px-3 py-2 text-right tabular-nums">{rate(value) ?? '—'}</td>)}
          </tr>)}</tbody>
        </table>
      </div> : <Empty title="没有匹配的模型" description={search ? '换个关键词再试。' : '目录里还没有可显示的模型。'} />}
      <ul className="grid gap-1.5 text-xs leading-5 text-muted-foreground">
        <li>单位为 USD / 百万 Token，按实际调用的上游模型名匹配；用了模型映射时，按映射后的名字计价。</li>
        <li>表里没有或待定价的模型可以正常调用，但不计入费用和费用上限。</li>
        <li>按时段计价的模型（如 DeepSeek 高峰价翻倍）统一按高峰价估算。</li>
        <li>上游折扣、包月套餐、人民币计价或第三方中转，都会让估算和实际帐单不同。</li>
      </ul>
    </div>
  </FormSheet>
}
