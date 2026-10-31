import { useId, useMemo, useRef, useState } from 'react'
import { X } from 'lucide-react'
import { Input } from './ui/input'
import { Button } from './ui/button'
import { isConcreteModel, isModelRule, splitModelTokens, modelScopeLabel } from '../lib/model-rules'

type ModelSuggestion = string | { value: string; label?: string }

export function ModelTagInput({ id, values, suggestions = [], disabled, allowPrefixes = true, placeholder = '输入模型名，回车添加', ariaLabel = '模型列表', onChange }: { id?: string; values: string[]; suggestions?: ModelSuggestion[]; disabled?: boolean; allowPrefixes?: boolean; placeholder?: string; ariaLabel?: string; onChange: (models: string[]) => void }) {
  const [draft, setDraft] = useState('')
  const [activeIndex, setActiveIndex] = useState(-1)
  const [focused, setFocused] = useState(false)
  const [error, setError] = useState('')
  // Backspace only undoes rules added in this session, never an arbitrary saved rule.
  const [recent, setRecent] = useState<string[]>([])
  const listboxId = useId()
  const root = useRef<HTMLDivElement>(null)
  // Suggestions are concrete models only; prefix rules come from the button below or typing `gpt-*`.
  const options = useMemo(() => [...new Map(suggestions.map(item => typeof item === 'string' ? { value: item, label: item } : item)
    .filter(item => isConcreteModel(item.value))
    .map(item => [item.value, { ...item, label: item.label || item.value }])).values()], [suggestions])
  const query = draft.trim()
  const search = query.replace(/\*$/, '').toLocaleLowerCase()
  const matching = options.filter(item => !values.includes(item.value) && (!search || `${item.value} ${item.label}`.toLocaleLowerCase().includes(search)))
  const displayed = matching.slice(0, 8)
  const prefix = query.endsWith('*') ? query : `${query}*`
  // Same case-sensitive prefix as the rule button, so both actions cover one set.
  const concreteMatches = options.filter(item => !values.includes(item.value) && isConcreteModel(item.value) && item.value.startsWith(prefix.slice(0, -1)))
  const showDropdown = !disabled && focused && displayed.length > 0
  const add = (raw: string) => {
    const parts = splitModelTokens(raw)
    if (parts.some(item => allowPrefixes ? !isModelRule(item) : !isConcreteModel(item))) {
      setError(allowPrefixes ? '仅支持具体名称或一个末尾 *，例如 gpt-*。' : '请填写具体模型名称，探测不支持前缀规则。')
      return
    }
    const added = parts.filter(item => !values.includes(item))
    onChange([...new Set([...values, ...parts])]); setRecent(items => [...items, ...added]); setDraft(''); setActiveIndex(-1); setFocused(false); setError('')
  }
  // Leaving the field only closes suggestions; raw text is added with Enter.
  return <div ref={root} className="relative grid gap-2" onBlur={event => { if (!root.current?.contains(event.relatedTarget)) { setFocused(false); setActiveIndex(-1) } }}>
    <div className="relative flex min-h-11 flex-wrap items-center gap-2 rounded-lg border border-input bg-background px-2 py-1 focus-within:border-ring">
      {values.map(model => <span key={model} className="inline-flex max-w-full items-center gap-1 rounded-md bg-primary/10 px-2 text-xs">
        <span className="max-w-[220px] truncate">{model}</span>{model.endsWith('*') && <span className="text-muted-foreground">前缀</span>}
        <button type="button" className="flex size-10 items-center justify-center rounded-sm hover:bg-primary/15 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" aria-label={`移除模型 ${model}`} onClick={() => onChange(values.filter(item => item !== model))} disabled={disabled}><X aria-hidden="true" className="size-3" /></button>
      </span>)}
      <Input id={id} aria-label={ariaLabel} role="combobox" aria-autocomplete="list" aria-expanded={showDropdown} aria-controls={listboxId} aria-activedescendant={showDropdown && activeIndex >= 0 ? `${listboxId}-${activeIndex}` : undefined}
        className="h-10 min-w-[180px] flex-1 border-0 px-1 shadow-none focus-visible:ring-0" value={draft} placeholder={placeholder} disabled={disabled}
        onFocus={() => setFocused(true)} onChange={event => { setDraft(event.target.value); setActiveIndex(-1); setError(''); setFocused(true) }}
        onKeyDown={event => {
          if (showDropdown && (event.key === 'ArrowDown' || event.key === 'ArrowUp')) {
            event.preventDefault(); setActiveIndex(index => event.key === 'ArrowDown' ? (index + 1) % displayed.length : (index <= 0 ? displayed.length - 1 : index - 1))
          } else if (event.key === 'Escape') { setFocused(false); setActiveIndex(-1) }
          else if (event.key === 'Enter' || event.key === ',') { event.preventDefault(); add(activeIndex >= 0 && showDropdown ? displayed[activeIndex].value : draft) }
          else if (event.key === 'Backspace' && !draft) {
            const last = [...recent].reverse().find(item => values.includes(item))
            if (last) { onChange(values.filter(item => item !== last)); setRecent(items => items.slice(0, items.lastIndexOf(last))) }
          }
        }} />
    </div>
    {focused && !disabled && (showDropdown || query) && <div className="absolute inset-x-0 top-full z-50 mt-1 overflow-hidden rounded-md border border-border bg-popover">
      {showDropdown && <div id={listboxId} role="listbox" tabIndex={-1} aria-label="匹配的模型" className="max-h-48 overflow-auto p-1">
        {displayed.map((item, index) => <button key={item.value} id={`${listboxId}-${index}`} type="button" role="option" aria-selected={index === activeIndex} tabIndex={-1}
          className={`flex min-h-10 w-full items-center justify-between gap-3 rounded-sm px-2.5 py-2 text-left text-sm ${index === activeIndex ? 'bg-accent' : 'hover:bg-accent/60'}`}
          onMouseDown={event => event.preventDefault()} onClick={() => add(item.value)}><span className="truncate">{item.label}</span><span className="shrink-0 text-xs text-muted-foreground">{item.value}</span></button>)}
      </div>}
    {query && <div className="flex flex-wrap gap-2 border-t border-border p-2">
      {!!concreteMatches.length && <Button type="button" variant="outline" size="sm" disabled={disabled} onMouseDown={event => event.preventDefault()} onClick={() => add(concreteMatches.map(item => item.value).join(','))}>添加当前匹配的 {concreteMatches.length} 个模型</Button>}
      {allowPrefixes && isModelRule(prefix) && !values.includes(prefix) && <Button type="button" variant="outline" size="sm" disabled={disabled} onMouseDown={event => event.preventDefault()} onClick={() => add(prefix)}>添加前缀规则 {prefix}</Button>}
    </div>}
    </div>}
    {error ? <p role="alert" className="text-xs text-destructive">{error}</p> : !focused && query && <p className="text-xs text-muted-foreground">「{query}」尚未添加，按回车添加或清空输入。</p>}
  </div>
}

export function ModelScope({ rules, mode, known = [], kind = '已知' }: { rules: string[]; mode: string; known?: string[]; kind?: string }) {
  return <div className="grid gap-1 text-sm"><span>{modelScopeLabel(rules, mode)}</span><details className="text-xs text-muted-foreground"><summary className="cursor-pointer py-1">{kind} {known.length} 个</summary><div className="mt-1 max-h-40 max-w-64 overflow-auto break-all rounded-md bg-muted/40 p-2">{known.length ? known.map(model => <div key={model} className="py-0.5 font-mono">{model}</div>) : '尚无已知模型；前缀规则或全部转发仍可接受符合条件的具体名称。'}</div></details></div>
}
