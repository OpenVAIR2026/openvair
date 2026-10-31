import { useState, type FormEvent, type ReactNode } from 'react'
import { useMutation, useQuery } from '@tanstack/react-query'
import { ArrowRight, CheckCircle2, KeyRound, LoaderCircle, Plus, ShieldCheck, Trash2 } from 'lucide-react'
import { api, jsonBody, type Channel, type ChannelSource, type QuotaEvent, type SourceKey, type SourceKeyStatus } from '../lib/api'
import { queryClient, useUser } from '../lib/queries'
import { usageCost } from '../lib/usage-metrics'
import { Button } from './ui/button'
import { Input } from './ui/input'
import { Badge, Confirm, ErrorBox, Field, FormSheet, Health, Panel } from './common'
import { ModelSelect } from './model-select'
import { QuotaHistory } from './quota-history'

// Short labels keep the status column scannable; the reason is in the tooltip.
const statusLabels: Record<SourceKeyStatus, [string, 'success' | 'warning' | 'danger', string]> = {
  active: ['正常', 'success', '来源 Key 可用'],
  paused: ['已暂停', 'warning', '来源 Key 已被所有者停用'],
  exhausted_daily: ['已用完', 'warning', '来源 Key 今日额度已用完，次日恢复'],
  exhausted_total: ['已用完', 'danger', '来源 Key 总额度已用完，需所有者提高上限或重置'],
  revoked: ['已失效', 'danger', '来源 Key 已被撤销'],
}

/** Status of a VAIR key channel whose source key cannot serve requests now, or which is disabled. */
export function SourceStatus({ source, enabled }: { source: ChannelSource | null; enabled: boolean }) {
  if (!enabled) return <Health enabled={false} status="unknown" />
  const [label, tone, reason] = statusLabels[source?.status ?? 'revoked']
  return <span title={reason}><Badge tone={tone}><span className="size-1.5 rounded-full bg-current" />{label}</Badge></span>
}

// Matches the API Key page: cents for ordinary amounts, more digits below one cent.
const usd = (value: number) => value === 0 || value >= 0.01 ? `$${value.toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 })}` : usageCost(value)
const millions = (value: number) => `${(value / 1_000_000).toLocaleString('zh-CN', { maximumFractionDigits: 2 })}M`

/** Limits of a source key, with usage in the gateway's windows when known. */
export function sourceLimits(source: ChannelSource | Partial<SourceKey>) {
  const used = 'status' in source ? source : undefined
  const progress = (limit: number, spent: number | undefined, format: (value: number) => string) => spent === undefined ? format(limit) : `${format(spent)} / ${format(limit)}`
  const parts = [
    source.cost_limit_daily_usd != null && `今日 ${progress(source.cost_limit_daily_usd, used?.used_cost_today_usd, usd)}`,
    source.token_limit_daily != null && `今日 ${progress(source.token_limit_daily, used?.used_tokens_today, millions)} Token`,
    source.cost_limit_total_usd != null && `累计 ${progress(source.cost_limit_total_usd, used?.used_cost_total_usd, usd)}`,
    source.requests_per_minute != null && `${source.requests_per_minute} 次/分钟`,
    source.concurrency_limit != null && `并发 ${source.concurrency_limit}`,
  ].filter(Boolean)
  return parts.length ? parts.join(' · ') : '不限'
}

function sourceModels(source: Partial<SourceKey>) {
  if (source.models?.length) return source.models.join(', ')
  if (source.allowed_models?.length) return source.allowed_models.join(', ')
  return source.models ? '来源帐号暂无可用模型' : '—'
}

/** Read-only facts about the key behind a VAIR key channel. */
export function SourceKeyDetails({ source, title = '来源信息', action }: { source: ChannelSource | Partial<SourceKey>; title?: string; action?: ReactNode }) {
  const status = 'status' in source ? statusLabels[source.status] : null
  const rows: [string, ReactNode][] = [
    ...(status && 'status' in source ? [['状态', <span className="inline-flex flex-wrap items-center gap-2"><Badge tone={status[1]}><span className="size-1.5 rounded-full bg-current" />{status[0]}</Badge>{source.status !== 'active' && <span className="text-muted-foreground">{status[2]}</span>}</span>] as [string, ReactNode]] : []),
    ['来源帐号', source.account_name ?? '—'],
    ['Key 名称', source.key_name ? `${source.key_name}（${source.key_prefix}…）` : '—'],
    ['可用模型', sourceModels(source)],
    ['额度', sourceLimits(source)],
    ['协议', 'OpenAI、Anthropic、Gemini 均可'],
  ]
  return <div className="grid gap-2 rounded-lg border border-border bg-muted/20 p-4 text-sm">
    <div className="flex items-center justify-between gap-3"><p className="font-medium">{title}</p>{action}</div>
    <dl className="grid grid-cols-[5.5rem_minmax(0,1fr)] items-center gap-x-3 gap-y-1.5">
      {rows.map(([label, value]) => <div key={label} className="contents"><dt className="text-muted-foreground">{label}</dt><dd className="break-all">{value}</dd></div>)}
    </dl>
  </div>
}

/**
 * Source details of a VAIR key channel in its editor. The channel can be
 * pointed at another key at any time.
 */
export function VairKeySource({ channel, onReplaced }: { channel: Channel; onReplaced: () => Promise<void> }) {
  const [open, setOpen] = useState(false); const [secret, setSecret] = useState('')
  const resolve = useMutation({ mutationFn: (value: string) => api<SourceKey>('/channels/vair-key/resolve', jsonBody({ secret: value })) })
  const replace = useMutation({ mutationFn: () => api(`/channels/${channel.id}/vair-key`, jsonBody({ secret: secret.trim() })), onSuccess: async () => { close(); await onReplaced() } })
  const close = () => { setOpen(false); setSecret(''); resolve.reset(); replace.reset() }
  const resolved = resolve.data && resolve.variables === secret.trim() ? resolve.data : null
  const identify = () => { const value = secret.trim(); if (value && value !== resolve.variables) resolve.mutate(value) }
  const source = channel.source ?? { status: 'revoked' as const }
  const action = !open && <Button type="button" variant="outline" size="sm" onClick={() => setOpen(true)}><KeyRound aria-hidden="true" className="size-4" />更换 Key</Button>
  const quota = useQuery({ queryKey: ['channel-quota', channel.id, source.key_prefix], queryFn: () => api<QuotaEvent[]>(`/channels/${channel.id}/quota-events`), enabled: source.status !== 'revoked' })
  return <div className="grid gap-3">
    <SourceKeyDetails source={source} action={action} />
    {source.status !== 'revoked' && (source.cost_limit_total_usd != null || !!quota.data?.length) && <section className="grid gap-2" aria-label="来源 Key 额度记录">
      <p className="text-sm font-medium">额度记录</p>
      <QuotaHistory events={quota.data} pending={quota.isPending} error={quota.error} retry={() => quota.refetch()} initial={3} empty="来源 Key 暂无额度记录" />
    </section>}
    {open && <div className="grid gap-3 rounded-lg border border-border p-4">
      <Field label="新的 VAIR Key" hint="更换后保留渠道的名称、优先级、模型映射和监控设置，旧 Key 的检查结果会清除。">{id => <Input id={id} type="password" autoComplete="off" spellCheck={false} value={secret} onChange={event => setSecret(event.target.value)} onBlur={identify} onKeyDown={event => { if (event.key === 'Enter') { event.preventDefault(); identify() } }} placeholder="vair_…" disabled={replace.isPending} />}</Field>
      {resolve.isPending && <p role="status" className="flex items-center gap-1.5 text-sm text-muted-foreground"><LoaderCircle aria-hidden="true" className="size-4 animate-spin" />正在识别…</p>}
      {resolved && <SourceKeyDetails source={resolved} title="新 Key" />}
      <ErrorBox error={resolve.error ?? replace.error} />
      <div className="flex flex-wrap justify-end gap-2">
        <Button type="button" variant="outline" size="sm" disabled={replace.isPending} onClick={close}>取消</Button>
        {resolved
          ? <Button type="button" size="sm" disabled={replace.isPending} onClick={() => replace.mutate()}>{replace.isPending ? '更换中…' : '确认更换'}</Button>
          : <Button type="button" size="sm" disabled={resolve.isPending || !secret.trim()} onClick={identify}>识别 Key</Button>}
      </div>
    </div>}
  </div>
}

/** Adds another account's VAIR key, pasted by the user, as a channel. */
export function VairKeyChannelForm({ onCancel, onCreated }: { onCancel: () => void; onCreated: () => void }) {
  const user = useUser()
  const [name, setName] = useState(''); const [secret, setSecret] = useState(''); const [priority, setPriority] = useState('100')
  const resolve = useMutation({ mutationFn: (value: string) => api<SourceKey>('/channels/vair-key/resolve', jsonBody({ secret: value })), onSuccess: source => setName(current => current || `${source.account_name} · ${source.key_name}`) })
  const create = useMutation({ mutationFn: () => api<{ id: string }>('/channels/vair-key', jsonBody({ name: name.trim(), secret: secret.trim(), priority: Number(priority) })), onSuccess: async () => { await queryClient.invalidateQueries({ queryKey: ['channels', user.account_id] }); onCreated() } })
  const resolved = resolve.data && resolve.variables === secret.trim() ? resolve.data : null
  const busy = resolve.isPending || create.isPending
  const identify = () => { const value = secret.trim(); if (value && value !== resolve.variables) resolve.mutate(value) }
  const submit = (event: FormEvent<HTMLFormElement>) => { event.preventDefault(); if (resolved && name.trim() && !busy) create.mutate() }
  return <form className="grid gap-4" onSubmit={submit}>
    <Field label="VAIR Key" hint="粘贴其他 VAIR 帐号提供给你的 Key。只保存引用，不保存明文。">{id => <Input id={id} type="password" autoComplete="off" spellCheck={false} value={secret} onChange={event => setSecret(event.target.value)} onBlur={identify} placeholder="vair_…" disabled={create.isPending} />}</Field>
    {resolve.isPending && <p role="status" className="flex items-center gap-1.5 text-sm text-muted-foreground"><LoaderCircle aria-hidden="true" className="size-4 animate-spin" />正在识别…</p>}
    {resolved && <SourceKeyDetails source={resolved} title="已识别" />}
    <Field label="渠道名称">{id => <Input id={id} autoComplete="off" value={name} onChange={event => setName(event.target.value)} placeholder="Alice 实验室 · VAIR Key" disabled={create.isPending} />}</Field>
    <Field label="渠道优先级" hint="数值越小越优先。来源 Key 不可用时自动切换到其他渠道。">{id => <Input id={id} type="number" min="0" value={priority} onChange={event => setPriority(event.target.value)} disabled={create.isPending} />}</Field>
    <ErrorBox error={resolve.error ?? create.error} />
    <div className="sticky bottom-0 z-10 flex flex-wrap items-center justify-between gap-3 border-t border-border bg-background py-4">
      <Button type="button" variant="outline" disabled={busy || !secret.trim()} onClick={identify}><KeyRound aria-hidden="true" className="size-4" />识别 Key</Button>
      <div className="flex gap-2"><Button type="button" variant="outline" disabled={create.isPending} onClick={onCancel}>取消</Button><Button type="submit" disabled={busy || !resolved || !name.trim()}>{create.isPending ? '创建中…' : '创建渠道'}</Button></div>
    </div>
  </form>
}

type Mapping = { public_model: string; upstream_model: string }

/**
 * Editor for a VAIR key channel. Only routing settings live here; models,
 * limits and the upstream belong to the source key's owner.
 */
export function VairChannelEditor({ channel, onClose, onSaved }: { channel: Channel; onClose: () => void; onSaved: () => Promise<void> }) {
  const user = useUser()
  const initialMappings = Object.entries(channel.model_mapping ?? {}).map(([public_model, upstream_model]) => ({ public_model, upstream_model }))
  const [current, setCurrent] = useState(channel)
  const [name, setName] = useState(channel.name); const [priority, setPriority] = useState(String(channel.priority))
  const [testModel, setTestModel] = useState(channel.test_model ?? ''); const [mappings, setMappings] = useState<Mapping[]>(initialMappings)
  const [invalid, setInvalid] = useState(''); const [discard, setDiscard] = useState(false)
  const filled = mappings.map(item => ({ public_model: item.public_model.trim(), upstream_model: item.upstream_model.trim() })).filter(item => item.public_model && item.upstream_model)
  const save = useMutation({ mutationFn: () => api(`/channels/${channel.id}`, jsonBody({ name: name.trim(), priority: Number(priority), test_model: testModel.trim() || null, model_mapping: Object.fromEntries(filled.map(item => [item.public_model, item.upstream_model])) }, 'PATCH')), onSuccess: async () => { await onSaved(); onClose() } })
  const check = useMutation({ mutationFn: () => api<{ healthy: boolean; message: string }>(`/channels/${channel.id}/check`, { method: 'POST' }), onSuccess: onSaved })
  const busy = save.isPending || check.isPending
  const dirty = name !== channel.name || priority !== String(channel.priority) || testModel !== (channel.test_model ?? '') || JSON.stringify(mappings) !== JSON.stringify(initialMappings)
  const requestClose = () => { if (!busy) { if (dirty) setDiscard(true); else onClose() } }
  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (busy) return
    const names = filled.map(item => item.public_model)
    if (!name.trim()) setInvalid('请填写渠道名称')
    else if (!(Number(priority) >= 0)) setInvalid('优先级必须是非负整数')
    else if (new Set(names).size !== names.length) setInvalid('同一渠道的对外模型名不能重复')
    else { setInvalid(''); save.mutate() }
  }
  const replaced = async () => { await onSaved(); const fresh = queryClient.getQueryData<Channel[]>(['channels', user.account_id])?.find(item => item.id === channel.id); if (fresh) setCurrent(fresh) }
  return <>
    <FormSheet open onOpenChange={open => { if (!open) requestClose() }} title="编辑渠道" description="可用模型和额度由来源 Key 的所有者决定。">
      <form className="grid gap-4" onSubmit={submit}>
        <Field label="渠道名称">{id => <Input id={id} autoComplete="off" value={name} onChange={event => setName(event.target.value)} disabled={busy} />}</Field>
        <VairKeySource channel={current} onReplaced={replaced} />
        <Field label="渠道优先级" help="数值越小越优先。">{id => <Input id={id} type="number" min="0" value={priority} onChange={event => setPriority(event.target.value)} disabled={busy} />}</Field>
        <Field label="测试模型（选填）" help="留空使用来源 Key 的第一个可用模型。每次检查计入来源 Key 的额度。">{id => <ModelSelect id={id} name="test_model" value={testModel} onChange={setTestModel} models={(current.source?.models ?? []).concat(filled.map(item => item.public_model))} disabled={busy} />}</Field>
        <div className="grid gap-3 rounded-lg border border-border p-4">
          <div className="flex items-start justify-between gap-3"><div><p className="text-sm font-medium">模型映射</p><p className="mt-1 text-xs leading-5 text-muted-foreground">把应用使用的模型名映射到来源 Key 可用的模型；没有映射时原样交给来源 Key。</p></div><Button type="button" variant="outline" size="sm" disabled={busy} onClick={() => setMappings(items => [...items, { public_model: '', upstream_model: '' }])}><Plus aria-hidden="true" className="size-4" />添加映射</Button></div>
          {mappings.map((item, index) => <div key={index} className="grid grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)_auto] items-center gap-2">
            <Input aria-label="对外模型名" spellCheck={false} placeholder="对外模型名" value={item.public_model} disabled={busy} onChange={event => setMappings(items => items.map((value, i) => i === index ? { ...value, public_model: event.target.value } : value))} />
            <ArrowRight aria-hidden="true" className="size-4 text-muted-foreground" />
            <Input aria-label="来源 Key 模型名" spellCheck={false} placeholder="来源 Key 模型名" value={item.upstream_model} disabled={busy} onChange={event => setMappings(items => items.map((value, i) => i === index ? { ...value, upstream_model: event.target.value } : value))} />
            <Button type="button" variant="ghost" size="icon" aria-label="删除模型映射" disabled={busy} onClick={() => setMappings(items => items.filter((_, i) => i !== index))}><Trash2 aria-hidden="true" className="size-4 text-destructive" /></Button>
          </div>)}
        </div>
        {invalid && <p role="alert" className="text-sm text-destructive">{invalid}</p>}
        <ErrorBox error={save.error ?? check.error} />
        {check.data && <p role="status" className={`flex items-center gap-1.5 text-sm ${check.data.healthy ? 'text-success' : 'text-destructive'}`}>{check.data.healthy ? <><CheckCircle2 aria-hidden="true" className="size-4" />检查通过</> : `检查未通过：${check.data.message}`}</p>}
        <div className="sticky bottom-0 z-10 flex flex-wrap items-center justify-between gap-3 border-t border-border bg-background py-4">
          <Button type="button" variant="outline" disabled={busy} onClick={() => check.mutate()}>{check.isPending ? <LoaderCircle aria-hidden="true" className="size-4 animate-spin" /> : <ShieldCheck aria-hidden="true" className="size-4" />}{check.isPending ? '检查中…' : '检查渠道'}</Button>
          <div className="flex gap-2"><Button type="button" variant="outline" disabled={busy} onClick={requestClose}>取消</Button><Button type="submit" disabled={busy}>{save.isPending ? '保存中…' : '保存设置'}</Button></div>
        </div>
      </form>
    </FormSheet>
    <Confirm open={discard} onOpenChange={setDiscard} title="放弃未保存的修改？" description="当前修改尚未保存。" pending={false} onConfirm={onClose} />
  </>
}
