import { useEffect, useMemo, useState } from 'react'
import { useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import { useMutation, useQuery } from '@tanstack/react-query'
import { ArrowRight, BarChart3, Check, CircleAlert, Copy, CopyPlus, Folder, FolderPlus, History, Link2, MoreHorizontal, Pencil, Plug, Plus, Power, TimerReset, Eraser, Trash2, X } from 'lucide-react'
import { Link, useSearchParams } from 'react-router-dom'
import { api, formatDate, jsonBody, type Channel, type KeyGroup, type QuotaEvent, type VirtualKey } from '../lib/api'
import { queryClient, useChannels, useKeyGroups, useKeys, useModelSuggestions, useOrigin, useUser } from '../lib/queries'
import { keyGroupSchema, keyLimitFields, keySchema, NEW_GROUP, type KeyGroupValues, type KeyValues } from '../lib/schemas'
import { Button } from '../components/ui/button'
import { Checkbox } from '../components/ui/checkbox'
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from '../components/ui/dropdown-menu'
import { Input } from '../components/ui/input'
import { Badge, Choice, Confirm, DataTable, Empty, ErrorBox, Field, FormSheet, HelpTip, Loading, PageHeader, Panel, Secret } from '../components/common'
import { copyText } from '../lib/clipboard'
import { rulesOverlap, splitModelTokens } from '../lib/model-rules'
import { ModelTagInput } from '../components/model-tags'
import { usageCost } from '../lib/usage-metrics'
import { ConnectGuide, usableKeys } from '../components/connect-guide'
import { DesktopPromo } from '../components/desktop-promo'
import { CLOUD } from '../lib/edition'
import { QuotaHistory } from '../components/quota-history'

type LimitField = typeof keyLimitFields[number]

const limitFields: Record<LimitField, { menu: string; label: string; help: string; min: string }> = {
  token_limit_daily: { menu: '每日 Token', label: '每日 Token 上限（百万）', help: '今日输入与输出 Token 合计达到上限后暂停新请求。按 UTC 自然日（北京时间 08:00）重置。', min: '0.001' },
  cost_limit_daily_usd: { menu: '每日费用', label: '每日费用上限（USD）', help: '已计价金额达到上限后暂停新请求，不预扣进行中请求的费用。按 UTC 自然日（北京时间 08:00）重置。各模型的单价见「使用分析 → 模型单价」。', min: '0.0001' },
  cost_limit_total_usd: { menu: '总费用', label: '总费用上限（USD）', help: '从 Key 创建起累计的已计价金额达到上限后暂停新请求，不预扣进行中请求的费用。调高上限（编辑时可直接增加额度）或重置用量即可恢复。各模型的单价见「使用分析 → 模型单价」。', min: '0.0001' },
}

const defaults: KeyValues = { name: '', holder: '', group_id: '', new_group: '', channel_mode: 'all', channel_ids: [], model_mode: 'allowlist', allowed_models: '', requests_per_minute: '60', token_limit_daily: '', cost_limit_daily_usd: '', cost_limit_total_usd: '', active_limits: [], concurrency_limit: '4' }

type Scope = Pick<KeyValues, 'channel_mode' | 'channel_ids' | 'model_mode' | 'allowed_models'>

function scopePayload(values: Scope) {
  return {
    channel_mode: values.channel_mode,
    channel_ids: values.channel_mode === 'selected' ? values.channel_ids : [],
    model_mode: values.model_mode,
    allowed_models: [...new Set(splitModelTokens(values.allowed_models))],
  }
}

function groupScope(group: KeyGroup): Scope {
  return { channel_mode: group.channel_mode, channel_ids: group.channel_ids, model_mode: group.model_mode, allowed_models: group.allowed_models.join(', ') }
}

function toPayload(values: KeyValues) {
  const active = (field: LimitField) => values.active_limits.includes(field) && values[field] ? Number(values[field]) : null
  const tokenLimit = active('token_limit_daily')
  return {
    name: values.name,
    holder: values.holder || null,
    ...scopePayload(values),
    requests_per_minute: values.requests_per_minute ? Number(values.requests_per_minute) : null,
    token_limit_daily: tokenLimit != null ? Math.round(tokenLimit * 1_000_000) : null,
    cost_limit_daily_usd: active('cost_limit_daily_usd'),
    cost_limit_total_usd: active('cost_limit_total_usd'),
    concurrency_limit: Number(values.concurrency_limit),
  }
}

function toFormValues(key: VirtualKey): KeyValues {
  return {
    name: key.name,
    holder: key.holder ?? '',
    group_id: key.group_id ?? '',
    new_group: '',
    channel_mode: key.channel_mode,
    channel_ids: key.channel_ids,
    model_mode: key.model_mode,
    allowed_models: key.allowed_models.join(', '),
    requests_per_minute: key.requests_per_minute?.toString() ?? '',
    token_limit_daily: key.token_limit_daily != null ? (key.token_limit_daily / 1_000_000).toString() : '',
    cost_limit_daily_usd: key.cost_limit_daily_usd?.toString() ?? '',
    cost_limit_total_usd: key.cost_limit_total_usd?.toString() ?? '',
    active_limits: keyLimitFields.filter(field => key[field] != null),
    concurrency_limit: key.concurrency_limit.toString(),
  }
}

function tokens(value: number) {
  if (value >= 1_000_000) return `${(value / 1_000_000).toLocaleString('zh-CN', { maximumFractionDigits: 2 })}M`
  if (value >= 1_000) return `${(value / 1_000).toLocaleString('zh-CN', { maximumFractionDigits: 1 })}K`
  return value.toLocaleString('zh-CN')
}

function money(value: number) {
  return value === 0 || value >= 0.01 ? `$${value.toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 })}` : usageCost(value)
}

// Adds to the saved total cost limit right away and records it, with a note the key's users can read.
function TotalLimitIncrease({ keyItem, limit, onIncreased }: { keyItem: VirtualKey; limit: string; onIncreased: (limit: number) => Promise<void> }) {
  const [adding, setAdding] = useState(false)
  const [amount, setAmount] = useState('')
  const [note, setNote] = useState('')
  const saved = keyItem.cost_limit_total_usd
  const current = Number(limit) || 0
  const delta = Number(amount)
  const valid = amount !== '' && Number.isFinite(delta) && delta > 0
  const increase = useMutation({ mutationFn: () => api<{ cost_limit_total_usd: number }>(`/keys/${keyItem.id}/quota-increases`, jsonBody({ amount_usd: delta, note: note.trim() || null })), onSuccess: async result => { close(); await onIncreased(result.cost_limit_total_usd) } })
  const close = () => { setAdding(false); setAmount(''); setNote(''); increase.reset() }
  const add = () => { if (valid && !increase.isPending) increase.mutate() }
  // Increases build on the saved limit, so an unsaved edit has to be saved or undone first.
  const edited = saved == null || Math.abs(current - saved) > 1e-9
  return <div className="grid gap-2">
    <div className="flex items-center justify-between gap-3">
      <p className="text-xs text-muted-foreground tabular-nums">已用 {money(keyItem.used_cost_total_usd)} · 剩余 {money(Math.max(0, current - keyItem.used_cost_total_usd))}</p>
      {!adding && <Button type="button" variant="outline" size="sm" disabled={edited} title={edited ? '请先保存总费用上限，再增加额度' : '增加额度立即生效，并记入额度记录'} onClick={() => setAdding(true)}><Plus />增加额度</Button>}
    </div>
    {adding && <div className="grid gap-2 rounded-md border border-border p-3">
      <Input type="number" min="0.0001" step="any" autoFocus placeholder="增加金额（USD）" aria-label="增加总费用额度（USD）" value={amount} disabled={increase.isPending} onChange={event => setAmount(event.target.value)} onKeyDown={event => { if (event.key === 'Enter') { event.preventDefault(); add() } }} />
      <Input maxLength={200} placeholder="备注（选填，客户可见）" aria-label="增加额度备注" value={note} disabled={increase.isPending} onChange={event => setNote(event.target.value)} onKeyDown={event => { if (event.key === 'Enter') { event.preventDefault(); add() } }} />
      <p className="text-xs text-muted-foreground">确定后立即生效，不需要再保存设置。使用这把 Key 的人能看到金额和备注。</p>
      <ErrorBox error={increase.error} />
      <div className="flex justify-end gap-2">
        <Button type="button" variant="ghost" size="sm" disabled={increase.isPending} onClick={close}>取消</Button>
        <Button type="button" size="sm" disabled={!valid || increase.isPending} onClick={add}>{increase.isPending ? '增加中…' : valid ? `增加 ${money(delta)}` : '确定'}</Button>
      </div>
    </div>}
  </div>
}

/** The key's quota history, as its holders and channel users also see it. */
function QuotaSheet({ keyItem, onClose }: { keyItem: VirtualKey | null; onClose: () => void }) {
  const events = useQuery({ queryKey: ['key-quota', keyItem?.id], queryFn: () => api<QuotaEvent[]>(`/keys/${keyItem!.id}/quota-events`), enabled: !!keyItem })
  return <FormSheet open={!!keyItem} onOpenChange={value => { if (!value) onClose() }} title={`额度记录 · ${keyItem?.name ?? ''}`} description="总费用上限的每次增加、调整和用量重置。持有这把 Key 的人和把它用作渠道的帐号看到同样的记录和备注。">
    {keyItem && <div className="grid gap-4">
      <p className="text-sm tabular-nums">当前上限 {keyItem.cost_limit_total_usd == null ? '不限' : money(keyItem.cost_limit_total_usd)} · 已用 {money(keyItem.used_cost_total_usd)}{keyItem.cost_limit_total_usd != null && ` · 剩余 ${money(Math.max(0, keyItem.cost_limit_total_usd - keyItem.used_cost_total_usd))}`}</p>
      <QuotaHistory events={events.data} pending={events.isPending} error={events.error} retry={() => events.refetch()} />
    </div>}
  </FormSheet>
}

function reached(used: number, limit: number | null) {
  return limit != null && used >= limit
}

// Used amounts follow the gateway's windows: the UTC day and the key's lifetime.
// Every cell has the same two lines so rows keep one height with or without limits.
function Metric({ label, used, limit, format, reset }: { label: string; used: number; limit: number | null; format: (value: number) => string; reset?: string }) {
  const ratio = limit ? used / limit : 0
  const tone = ratio >= 1 ? { bar: 'bg-destructive', text: ' text-destructive' } : ratio >= 0.8 ? { bar: 'bg-warning', text: ' text-warning' } : { bar: 'bg-primary', text: '' }
  return <div className="grid justify-items-end gap-1.5 tabular-nums" title={reset}>
    <span className={`text-sm${tone.text}`}>{format(used)}</span>
    {limit != null
      ? <span className="flex items-center gap-2 text-xs text-muted-foreground"><span className="h-1 w-12 overflow-hidden rounded-full bg-secondary" role="meter" aria-label={label} aria-valuemin={0} aria-valuemax={limit} aria-valuenow={Math.min(used, limit)}><span className={`block h-full rounded-full ${tone.bar}`} style={{ width: `${Math.min(100, ratio * 100)}%` }} /></span>/ {format(limit)}</span>
      : <span className="text-xs text-muted-foreground">不限</span>}
  </div>
}

function KeyScope({ keyItem: key, channels }: { keyItem: VirtualKey; channels: Channel[] | undefined }) {
  const names = key.channel_ids.map(id => channels?.find(channel => channel.id === id)?.name).filter(Boolean)
  const models = key.model_mode === 'all' ? '全部模型' : key.allowed_models.join('、')
  return <div className="grid max-w-60 gap-1.5">
    {key.channel_mode === 'all' ? <span className="text-sm">全部渠道</span>
      : key.channel_ids.length ? <span className="truncate text-sm" title={names.join('、')}>{names.length ? names.join('、') : `${key.channel_ids.length} 个渠道`}</span>
      : <span className="inline-flex items-center gap-1 text-sm text-warning"><CircleAlert aria-hidden="true" className="size-3.5" />未接入可用渠道</span>}
    <span className="truncate text-xs text-muted-foreground" title={key.known_models.length ? `当前可路由 ${key.known_models.length} 个模型：${key.known_models.join('、')}` : '当前没有可路由的模型'}>{models}</span>
  </div>
}

type ResetScope = 'today' | 'all'

const resetCopy: Record<ResetScope, { title: string; description: (key: VirtualKey) => string }> = {
  today: { title: '重置今日用量？', description: key => `「${key.name}」今日已用 ${tokens(key.used_tokens_today)} Token、${money(key.used_cost_today_usd)}，将从现在重新计算，每日上限随之恢复；累计费用不变。调用记录和使用分析数据会保留。` },
  all: { title: '重置全部用量？', description: key => `「${key.name}」今日已用 ${tokens(key.used_tokens_today)} Token、${money(key.used_cost_today_usd)}，累计 ${money(key.used_cost_total_usd)}，都将从现在重新计算，每日和总费用上限随之恢复。调用记录和使用分析数据会保留。` },
}

function resetSince(value: string | null, daily: boolean, label: string) {
  if (!value) return undefined
  // A daily reset from an earlier day no longer affects today's window.
  if (daily && new Date(value).toISOString().slice(0, 10) !== new Date().toISOString().slice(0, 10)) return undefined
  return `${label}已于 ${formatDate(value)} 重置，从该时间起统计`
}

function usedToday(key: VirtualKey) {
  return key.used_tokens_today > 0 || key.used_cost_today_usd > 0
}

function limitReached(key: VirtualKey) {
  return reached(key.used_tokens_today, key.token_limit_daily) || reached(key.used_cost_today_usd, key.cost_limit_daily_usd) || reached(key.used_cost_total_usd, key.cost_limit_total_usd)
}

function KeyStatus({ keyItem }: { keyItem: VirtualKey }) {
  if (keyItem.revoked_at) return <Badge>已撤销</Badge>
  if (!keyItem.enabled) return <Badge>停用</Badge>
  if (limitReached(keyItem)) return <span title="已达到每日或总量上限，新请求会被拒绝；次日重置、调高上限或重置用量后恢复"><Badge tone="danger">已达上限</Badge></span>
  return <Badge tone="success">启用</Badge>
}

/** How many accounts use this key as a VAIR key channel. */
function KeyChannelCount({ keyItem }: { keyItem: VirtualKey }) {
  const { channel_count, shared_accounts, share_expires_at } = keyItem
  // Accounts that pasted the key are counted but not named.
  const named = shared_accounts.map(account => account.email ? `${account.name}（${account.email}）` : account.name)
  const others = channel_count - named.length
  const title = [`${channel_count} 个帐号把这把 Key 用作渠道，共用它的额度`, ...named.map(item => `· ${item}`), ...(others > 0 && named.length ? [`· 另有 ${others} 个帐号通过粘贴 Key 添加`] : [])].join('\n')
  return <>
    {channel_count > 0 && <span className="inline-flex shrink-0 items-center gap-1" title={title}><Link2 aria-label="用作渠道的帐号数" className="size-3.5" />{channel_count}</span>}
    {share_expires_at && <span className="inline-flex shrink-0 items-center gap-1" title={`分享链接 ${formatDate(share_expires_at)} 前有效，被接受一次后失效`}><Link2 aria-hidden="true" className="size-3.5" />待接受</span>}
  </>
}

/** Issues a one-time link that adds the key to another account as a channel. */
function ShareKeySheet({ keyItem, onClose }: { keyItem: VirtualKey | null; onClose: () => void }) {
  const user = useUser()
  const [link, setLink] = useState<{ url: string; expires_at: string } | null>(null)
  const [copied, setCopied] = useState(false); const [copyError, setCopyError] = useState(false)
  const refresh = () => queryClient.invalidateQueries({ queryKey: ['keys', user.account_id] })
  const create = useMutation({ mutationFn: (key: VirtualKey) => api<{ code: string; expires_at: string }>(`/keys/${key.id}/share`, { method: 'POST' }), onSuccess: async result => { setLink({ url: `${window.location.origin}/share#${result.code}`, expires_at: result.expires_at }); setCopied(false); await refresh() } })
  const cancel = useMutation({ mutationFn: (key: VirtualKey) => api(`/keys/${key.id}/share`, { method: 'DELETE' }), onSuccess: async () => { setLink(null); await refresh() } })
  const busy = create.isPending || cancel.isPending
  const close = () => { if (busy) return; setLink(null); setCopied(false); setCopyError(false); create.reset(); cancel.reset(); onClose() }
  const copy = async (value: string) => { try { await copyText(value); setCopyError(false); setCopied(true) } catch { setCopyError(true) } }
  const pending = keyItem?.share_expires_at
  return <FormSheet open={!!keyItem} onOpenChange={value => { if (!value) close() }} title={`分享 API Key · ${keyItem?.name ?? ''}`} description="把链接发给对方，对方登录或注册后可把这把 Key 添加为渠道，不会看到 Key 明文。">
    {keyItem && <div className="grid gap-4 text-sm">
      <ul className="grid list-disc gap-1 pl-5 leading-6 text-muted-foreground">
        <li>链接只能使用一次，24 小时内有效；重新生成会使旧链接失效。</li>
        <li>对方的请求共用这把 Key 的模型范围、额度和限速，用量计入这把 Key。</li>
        <li>停用或撤销这把 Key 后，对方的渠道随之暂停或失效。</li>
      </ul>
      {link ? <Panel className="grid gap-3 border-primary/30 p-4">
        <p className="font-medium text-primary">分享链接已生成，只显示这一次</p>
        <code className="block break-all rounded-md bg-background p-3 text-sm">{link.url}</code>
        <p className="text-xs text-muted-foreground">{formatDate(link.expires_at)} 前有效</p>
        {copyError && <p role="alert" className="text-xs text-destructive">无法访问剪贴板，请手动选择并复制链接。</p>}
        <div><Button type="button" variant="outline" size="sm" onClick={() => copy(link.url)}>{copied ? <Check className="size-4 text-success" /> : <Copy className="size-4" />}{copied ? '已复制链接' : '复制链接'}</Button></div>
      </Panel> : pending && <p className="rounded-md border border-dashed border-border px-3 py-2.5 text-muted-foreground">已有一个待接受的链接，{formatDate(pending)} 前有效。链接只在生成时显示，如已丢失请重新生成。</p>}
      <ErrorBox error={create.error ?? cancel.error} />
      <div className="flex flex-wrap justify-end gap-2">
        {(link || pending) && <Button type="button" variant="outline" disabled={busy} onClick={() => cancel.mutate(keyItem)}>{cancel.isPending ? '取消中…' : '取消分享'}</Button>}
        <Button type="button" disabled={busy} onClick={() => create.mutate(keyItem)}>{create.isPending ? '生成中…' : link || pending ? '重新生成链接' : '生成分享链接'}</Button>
      </div>
    </div>}
  </FormSheet>
}

/** Names a channel can serve: its rules, mapped names and catalog matches, or a source key's models. */
function channelModels(channel: Channel) {
  return [...channel.models, ...Object.keys(channel.model_mapping ?? {}), ...channel.known_models, ...(channel.source?.models ?? [])]
}

/** Channel and model pickers, shared by the key and group forms. */
function ScopeFields({ value, onChange, errors, channels }: { value: Scope; onChange: (patch: Partial<Scope>) => void; errors: { channel_ids?: string; allowed_models?: string }; channels: Channel[] | undefined }) {
  const { channel_mode: channelMode, channel_ids: channelIds } = value
  const scopedChannels = useMemo(() => (channels ?? []).filter(channel => channelMode === 'all' || channelIds.includes(channel.id)), [channelIds, channelMode, channels])
  const passthrough = scopedChannels.some(channel => channel.model_mode === 'passthrough')
  const allSuggestions = useModelSuggestions(channels?.flatMap(channel => [...channel.models, ...Object.keys(channel.model_mapping ?? {})]) ?? [])
  // Passthrough channels accept any catalog model, so only narrow suggestions without them.
  const candidates = useMemo(() => channelMode === 'all' || passthrough ? allSuggestions : [...new Set(scopedChannels.flatMap(channelModels))].sort().map(value => ({ value, label: value })), [allSuggestions, channelMode, passthrough, scopedChannels])
  const rules = splitModelTokens(value.allowed_models)
  const unservable = channelMode === 'selected' && channels && !passthrough ? rules.filter(rule => !scopedChannels.some(channel => [...channel.models, ...Object.keys(channel.model_mapping ?? {})].some(other => rulesOverlap(rule, other)))) : []
  const allModels = value.model_mode === 'all'
  const toggleChannel = (id: string, checked: boolean) => onChange({ channel_ids: checked ? [...channelIds, id] : channelIds.filter(item => item !== id) })
  return <>
    <Field label="接入渠道" error={channelMode === 'all' ? undefined : errors.channel_ids} action={<label className="flex items-center gap-2 text-sm"><Checkbox checked={channelMode === 'all'} onCheckedChange={checked => onChange({ channel_mode: checked === true ? 'all' : 'selected' })} />允许全部渠道</label>}>{id => channelMode === 'all'
      ? <p id={id} className="rounded-md border border-dashed border-border px-3 py-2.5 text-sm text-muted-foreground">可使用帐号下所有渠道，新增渠道后自动可用。</p>
      : channels?.length ? <div id={id} role="group" aria-label="接入渠道" className="grid max-h-56 gap-0.5 overflow-y-auto rounded-md border border-border p-1.5">{channels.map(channel => <label key={channel.id} className="flex min-h-9 items-center gap-2.5 rounded-sm px-2 text-sm hover:bg-accent/50"><Checkbox checked={channelIds.includes(channel.id)} onCheckedChange={checked => toggleChannel(channel.id, checked === true)} aria-label={channel.name} /><span className="min-w-0 flex-1 truncate">{channel.name}</span><span className="text-xs text-muted-foreground">{channel.protocol}</span>{!channel.enabled && <Badge>停用</Badge>}</label>)}</div>
      : <p id={id} className="rounded-md border border-dashed border-border px-3 py-2.5 text-sm text-muted-foreground">暂无渠道，请先<Link to="/channels" className="text-primary hover:underline">添加上游渠道</Link>。</p>}</Field>
    <Field label="允许模型" error={allModels ? undefined : errors.allowed_models} help={allModels ? undefined : '支持具体名称或前缀，如 gpt-*。候选模型来自所选渠道。'} action={<label className="flex items-center gap-2 text-sm"><Checkbox checked={allModels} onCheckedChange={checked => onChange({ model_mode: checked === true ? 'all' : 'allowlist' })} />允许全部模型</label>}>{id => <>
      {allModels ? <p id={id} className="rounded-md border border-dashed border-border px-3 py-2.5 text-sm text-muted-foreground">{channelMode === 'all' ? '可调用帐号下所有可路由的模型，渠道新增模型后自动可用。' : '可调用所选渠道支持的全部模型。'}</p> : <ModelTagInput id={id} ariaLabel="允许模型" values={rules} suggestions={candidates} onChange={models => onChange({ allowed_models: models.join(', ') })} placeholder="输入模型名或前缀" />}
      {!allModels && unservable.length > 0 && <p className="text-xs text-warning">所选渠道中无可用模型：{unservable.join('、')}</p>}
    </>}</Field>
  </>
}

/** Plain-text summary of a group's scope, shown where member keys would pick their own. */
function GroupScopeSummary({ group, channels }: { group: KeyGroup; channels: Channel[] | undefined }) {
  const names = group.channel_ids.map(id => channels?.find(channel => channel.id === id)?.name).filter(Boolean)
  const channelText = group.channel_mode === 'all' ? '全部渠道' : names.length ? names.join('、') : `${group.channel_ids.length} 个渠道`
  const modelText = group.model_mode === 'all' ? '全部模型' : group.allowed_models.join('、')
  return <Field label="渠道和模型" hint="修改分组设置后，组内所有 Key 立即生效。">{id => <p id={id} className="rounded-md border border-dashed border-border px-3 py-2.5 text-sm">跟随分组：{channelText} · {modelText}</p>}</Field>
}

const blankGroup: KeyGroupValues = { name: '', channel_mode: 'all', channel_ids: [], model_mode: 'all', allowed_models: '' }

/** Checklist of the account's keys; picking a key from another group moves it here. */
function sameScope(key: VirtualKey, scope: ReturnType<typeof scopePayload>) {
  const same = (a: string[], b: string[]) => a.length === b.length && [...a].sort().every((item, index) => item === [...b].sort()[index])
  return key.channel_mode === scope.channel_mode && same(key.channel_mode === 'selected' ? key.channel_ids : [], scope.channel_ids)
    && key.model_mode === scope.model_mode && same(key.model_mode === 'allowlist' ? key.allowed_models : [], scope.model_mode === 'allowlist' ? scope.allowed_models : [])
}

/** Members first, then ungrouped keys, then keys that would move from another group. */
function GroupMembers({ group, scope, keys, groups, value, onChange }: { group: KeyGroup | null; scope: Scope; keys: VirtualKey[]; groups: KeyGroup[]; value: string[]; onChange: (ids: string[]) => void }) {
  const [search, setSearch] = useState('')
  const term = search.trim().toLowerCase()
  const rank = (key: VirtualKey) => !key.group_id ? 1 : group && key.group_id === group.id ? 0 : 2
  const shown = keys.filter(key => !term || key.name.toLowerCase().includes(term) || !!key.holder?.toLowerCase().includes(term) || key.key_prefix.toLowerCase().includes(term)).sort((a, b) => rank(a) - rank(b))
  const target = scopePayload(scope)
  const toggle = (id: string, checked: boolean) => onChange(checked ? [...value, id] : value.filter(item => item !== id))
  return <Field label="组内 Key" help="勾选的 Key 渠道和模型跟随分组；取消勾选的 Key 保留当前设置并变为未分组。" action={<span className="text-xs text-muted-foreground">已选 {value.length} 个</span>}>{id => keys.length
    ? <div id={id} role="group" aria-label="组内 Key" className="grid gap-1.5 rounded-md border border-border p-1.5">
      <Input aria-label="搜索 Key" placeholder="搜索名称或前缀…" className="h-8" value={search} onChange={event => setSearch(event.target.value)} />
      <div className="grid max-h-64 gap-0.5 overflow-y-auto">{shown.map(key => {
        const current = key.group_id && key.group_id !== group?.id ? groups.find(item => item.id === key.group_id) : undefined
        return <label key={key.id} className="flex min-h-9 items-center gap-2.5 rounded-sm px-2 text-sm hover:bg-accent/50"><Checkbox checked={value.includes(key.id)} onCheckedChange={checked => toggle(key.id, checked === true)} aria-label={key.name} /><span className="min-w-0 flex-1 truncate">{key.name}</span>{current && <span className="truncate text-xs text-muted-foreground">在：{current.name}</span>}<span className="font-mono text-xs text-muted-foreground">{key.key_prefix}</span>{value.includes(key.id) && rank(key) !== 0 && !sameScope(key, target) ? <HelpTip label={key.name}>加入后修改为本组设置</HelpTip> : <span className="size-6 shrink-0" />}</label>
      })}{!shown.length && <p className="px-2 py-3 text-center text-sm text-muted-foreground">没有匹配的 Key</p>}</div>
    </div>
    : <p id={id} className="rounded-md border border-dashed border-border px-3 py-2.5 text-sm text-muted-foreground">还没有 API Key，创建 Key 时可选择此分组。</p>}</Field>
}

function GroupSheet({ open, group, channels, keys, groups, onClose, onSaved, onDeleted }: { open: boolean; group: KeyGroup | null; channels: Channel[] | undefined; keys: VirtualKey[]; groups: KeyGroup[]; onClose: () => void; onSaved: (id: string) => void; onDeleted: () => void }) {
  const user = useUser()
  const form = useForm<KeyGroupValues>({ resolver: zodResolver(keyGroupSchema), defaultValues: blankGroup })
  const [confirming, setConfirming] = useState(false)
  const [members, setMembers] = useState<string[]>([])
  const refresh = () => Promise.all([queryClient.invalidateQueries({ queryKey: ['key-groups', user.account_id] }), queryClient.invalidateQueries({ queryKey: ['keys', user.account_id] })])
  const save = useMutation({
    mutationFn: async (values: KeyGroupValues) => {
      const body = { name: values.name, ...scopePayload(values), key_ids: members }
      if (group) { await api(`/keys/groups/${group.id}`, jsonBody(body, 'PATCH')); return group.id }
      return (await api<{ id: string }>('/keys/groups', jsonBody(body))).id
    },
    onSuccess: async id => { await refresh(); onSaved(id) },
  })
  const remove = useMutation({ mutationFn: (id: string) => api(`/keys/groups/${id}`, { method: 'DELETE' }), onSuccess: async () => { setConfirming(false); await refresh(); onDeleted() } })
  useEffect(() => { if (open) { form.reset(group ? { name: group.name, ...groupScope(group) } : blankGroup); setMembers(group ? keys.filter(key => key.group_id === group.id).map(key => key.id) : []); save.reset(); remove.reset() } }, [open, group])
  const setScope = (patch: Partial<Scope>) => {
    const options = { shouldDirty: true, shouldValidate: form.formState.isSubmitted || 'model_mode' in patch || 'allowed_models' in patch }
    if (patch.channel_mode) form.setValue('channel_mode', patch.channel_mode, options)
    if (patch.channel_ids) form.setValue('channel_ids', patch.channel_ids, options)
    if (patch.model_mode) form.setValue('model_mode', patch.model_mode, options)
    if (patch.allowed_models != null) form.setValue('allowed_models', patch.allowed_models, options)
  }
  const pending = save.isPending || remove.isPending
  return <>
    <FormSheet open={open} onOpenChange={value => { if (!value && !pending) onClose() }} title={group ? `编辑分组 · ${group.name}` : '新建分组'} description="组内 Key 的渠道和模型跟随分组；限额仍在每个 Key 上单独设置。">
      <form className="grid gap-4" onSubmit={form.handleSubmit(values => save.mutate(values))}>
        <Field label="分组名称" error={form.formState.errors.name?.message}>{id => <Input id={id} {...form.register('name')} placeholder="学生班" />}</Field>
        <ScopeFields value={form.watch()} onChange={setScope} errors={{ channel_ids: form.formState.errors.channel_ids?.message, allowed_models: form.formState.errors.allowed_models?.message }} channels={channels} />
        <GroupMembers group={group} scope={form.watch()} keys={keys} groups={groups} value={members} onChange={setMembers} />
        {members.length > 0 && <p className="text-xs text-muted-foreground">保存后对组内 {members.length} 个 Key 立即生效。</p>}
        <ErrorBox error={save.error} />
        <div className="flex items-center gap-2">
          {group && <Button type="button" variant="ghost" className="text-destructive" disabled={pending} onClick={() => { remove.reset(); setConfirming(true) }}><Trash2 />删除分组</Button>}
          <div className="ml-auto flex gap-2"><Button type="button" variant="outline" disabled={pending} onClick={onClose}>取消</Button><Button type="submit" disabled={pending}>{save.isPending ? '保存中…' : group ? '保存' : '创建分组'}</Button></div>
        </div>
      </form>
    </FormSheet>
    <Confirm open={confirming} onOpenChange={value => { if (!value && !remove.isPending) setConfirming(false) }} title={`删除分组「${group?.name ?? ''}」？`} description={group?.key_count ? `组内 ${group.key_count} 个 Key 不会被撤销或停用，它们保留当前的渠道和模型，并变为未分组。` : '分组中没有 Key。'} pending={remove.isPending} error={remove.error} onConfirm={() => { if (group) remove.mutate(group.id) }} />
  </>
}

/** The gateway address every client needs alongside a key; OpenAI-compatible clients add /v1. */
function GatewayAddress({ onGuide }: { onGuide?: () => void }) {
  const origin = useOrigin()
  const [copied, setCopied] = useState<string | null>(null)
  const [error, setError] = useState(false)
  const copy = async (value: string) => {
    try { await copyText(value); setError(false); setCopied(value); window.setTimeout(() => setCopied(current => current === value ? null : current), 1600) } catch { setError(true) }
  }
  const row = (label: string, value: string, button: string) => <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1">
    <span className="w-28 shrink-0 text-xs text-muted-foreground">{label}</span>
    <code className="min-w-0 break-all text-sm">{value}</code>
    <Button type="button" variant="ghost" size="icon-xs" aria-label={button} title={copied === value ? '已复制' : button} onClick={() => copy(value)}>{copied === value ? <Check className="size-3.5 text-success" /> : <Copy className="size-3.5" />}</Button>
  </div>
  return <Panel className="mb-5 flex flex-wrap items-center justify-between gap-3 p-4">
    <section className="grid min-w-0 gap-1.5" aria-label="接口地址">
      {row('接口地址', origin, '复制接口地址')}
      {row('OpenAI 兼容客户端', `${origin}/v1`, '复制 /v1 接口地址')}
      {error && <p role="alert" className="text-xs text-destructive">无法访问剪贴板，请手动选择并复制地址。</p>}
    </section>
    {onGuide && <Button type="button" variant="outline" size="sm" onClick={onGuide}>查看接入指南<ArrowRight /></Button>}
  </Panel>
}

export function KeysPage() {
  const user = useUser(); const query = useKeys(); const channels = useChannels(); const groups = useKeyGroups()
  const [open, setOpen] = useState(false); const [editing, setEditing] = useState<VirtualKey | null>(null); const [remove, setRemove] = useState<VirtualKey | null>(null); const [secret, setSecret] = useState(''); const [created, setCreated] = useState<{ id: string; value: string } | null>(null); const [guide, setGuide] = useState<{ id: string; name: string; created?: boolean } | null>(null); const [copiedId, setCopiedId] = useState<string | null>(null); const [copyError, setCopyError] = useState<Error | null>(null)
  const [copyingId, setCopyingId] = useState<string | null>(null)
  const [sharing, setSharing] = useState<VirtualKey | null>(null)
  const [quota, setQuota] = useState<VirtualKey | null>(null)
  const form = useForm<KeyValues>({ resolver: zodResolver(keySchema), defaultValues: defaults })
  const groupId = form.watch('group_id')
  const formGroup = groups.data?.find(group => group.id === groupId)
  const groupName = (key: VirtualKey) => groups.data?.find(group => group.id === key.group_id)?.name ?? ''
  const activeLimits = form.watch('active_limits')
  const totalLimit = form.watch('cost_limit_total_usd')
  const refresh = () => Promise.all([queryClient.invalidateQueries({ queryKey: ['keys', user.account_id] }), queryClient.invalidateQueries({ queryKey: ['key-groups', user.account_id] })])
  // A new group is created first with the form's scope; on a later failure the form keeps it selected.
  const withGroup = async (values: KeyValues) => {
    let group = values.group_id
    if (group === NEW_GROUP) {
      group = (await api<{ id: string }>('/keys/groups', jsonBody({ name: values.new_group, ...scopePayload(values) }))).id
      await queryClient.invalidateQueries({ queryKey: ['key-groups', user.account_id] })
      form.setValue('group_id', group); form.setValue('new_group', '')
    }
    return { ...toPayload(values), group_id: group || null }
  }
  const create = useMutation({ mutationFn: async (values: KeyValues) => api<{ id: string; key: string }>('/keys', jsonBody(await withGroup(values))), onSuccess: async (result, values) => { setOpen(false); setCreated({ id: result.id, value: result.key }); setGuide({ id: result.id, name: values.name, created: true }); await refresh() } })
  // An unchanged total limit is left out, so saving cannot undo an increase made elsewhere meanwhile.
  const update = useMutation({ mutationFn: async ({ key, values }: { key: VirtualKey; values: KeyValues }) => { const { cost_limit_total_usd, ...rest } = await withGroup(values); return api(`/keys/${key.id}`, jsonBody(cost_limit_total_usd === key.cost_limit_total_usd ? rest : { ...rest, cost_limit_total_usd }, 'PATCH')) }, onSuccess: async (_, { key }) => { await Promise.all([refresh(), queryClient.invalidateQueries({ queryKey: ['key-quota', key.id] })]); setOpen(false); setEditing(null) } })
  const toggle = useMutation({ mutationFn: (key: VirtualKey) => api(`/keys/${key.id}/enabled`, jsonBody({ enabled: !key.enabled }, 'PATCH')), onSuccess: refresh })
  const revoke = useMutation({ mutationFn: (id: string) => api(`/keys/${id}`, { method: 'DELETE' }), onSuccess: async () => { setRemove(null); await refresh() } })
  const [resetting, setResetting] = useState<{ key: VirtualKey; scope: ResetScope } | null>(null)
  const resetUsage = useMutation({ mutationFn: ({ key, scope }: { key: VirtualKey; scope: ResetScope }) => api(`/keys/${key.id}/usage-reset`, jsonBody({ scope })), onSuccess: async (_, { key }) => { await Promise.all([refresh(), queryClient.invalidateQueries({ queryKey: ['key-quota', key.id] })]); setResetting(null) } })
  const askReset = (key: VirtualKey, scope: ResetScope) => { resetUsage.reset(); setResetting({ key, scope }) }
  const pending = create.isPending || update.isPending
  const [searchParams, setSearchParams] = useSearchParams()
  // `?group=` is a group id, or `none` for keys outside every group.
  const groupFilter = searchParams.get('group') ?? ''
  const filterGroup = groups.data?.find(group => group.id === groupFilter)
  const rows = useMemo(() => (query.data ?? []).filter(key => !groupFilter || (groupFilter === 'none' ? !key.group_id : key.group_id === groupFilter)), [groupFilter, query.data])
  const setGroupFilter = (value: string) => setSearchParams(previous => { const next = new URLSearchParams(previous); if (value) next.set('group', value); else next.delete('group'); return next }, { replace: true })
  const [groupSheet, setGroupSheet] = useState<{ group: KeyGroup | null } | null>(null)
  const openForm = (values: KeyValues) => { form.reset(values); create.reset(); update.reset(); setEditing(null); setOpen(true) }
  // Creating while a group is shown puts the new key in that group.
  const showCreate = () => openForm(filterGroup ? { ...defaults, ...groupScope(filterGroup), group_id: filterGroup.id } : defaults)
  const copyConfig = (key: VirtualKey) => openForm({ ...toFormValues(key), name: '', holder: '' })
  const chooseGroup = (value: string) => {
    const group = groups.data?.find(item => item.id === value)
    // Members carry the group's scope, so leaving keeps it and joining copies it in.
    if (group) for (const [field, scope] of Object.entries(groupScope(group)) as [keyof Scope, never][]) form.setValue(field, scope)
    form.setValue('group_id', value === 'none' ? '' : value, { shouldDirty: true, shouldValidate: form.formState.isSubmitted })
  }
  const setScope = (patch: Partial<Scope>) => {
    const options = { shouldDirty: true, shouldValidate: form.formState.isSubmitted || 'model_mode' in patch || 'allowed_models' in patch }
    if (patch.channel_mode) form.setValue('channel_mode', patch.channel_mode, options)
    if (patch.channel_ids) form.setValue('channel_ids', patch.channel_ids, options)
    if (patch.model_mode) form.setValue('model_mode', patch.model_mode, options)
    if (patch.allowed_models != null) form.setValue('allowed_models', patch.allowed_models, options)
  }
  // `?create=1` opens the create form directly, e.g. from the overview's getting-started steps.
  useEffect(() => {
    if (!searchParams.has('create')) return
    showCreate()
    setSearchParams(previous => { const next = new URLSearchParams(previous); next.delete('create'); return next }, { replace: true })
  }, [searchParams])
  const increased = async (limit: number) => {
    if (!editing) return
    form.setValue('cost_limit_total_usd', String(limit), { shouldValidate: form.formState.isSubmitted })
    setEditing({ ...editing, cost_limit_total_usd: limit })
    await Promise.all([refresh(), queryClient.invalidateQueries({ queryKey: ['key-quota', editing.id] })])
  }
  const showEdit = (key: VirtualKey) => { openForm(toFormValues(key)); setEditing(key) }
  const addLimit = (field: LimitField) => form.setValue('active_limits', [...activeLimits, field], { shouldDirty: true })
  const removeLimit = (field: LimitField) => { form.setValue(field, '', { shouldDirty: true }); form.setValue('active_limits', activeLimits.filter(item => item !== field), { shouldDirty: true, shouldValidate: form.formState.isSubmitted }) }
  const copyKey = async (key: VirtualKey) => {
    setCopyingId(key.id); setCopyError(null)
    try {
      const result = await api<{ key: string }>(`/keys/${key.id}/reveal`, { method: 'POST', cache: 'no-store' })
      try {
        await copyText(result.key)
      } catch {
        setSecret(result.key)
        throw new Error(window.isSecureContext ? '无法访问剪贴板，请点上方的「复制完整 API Key」或手动复制。' : '页面通过 http:// 访问时浏览器不允许自动复制，请点上方的「复制完整 API Key」。')
      }
      setCopyError(null); setCopiedId(key.id); window.setTimeout(() => setCopiedId(current => current === key.id ? null : current), 1600)
    } catch (error) {
      setCopyError(error instanceof Error ? error : new Error('复制 API Key 失败，请重试。'))
    } finally {
      setCopyingId(null)
    }
  }
  const channelNames = (key: VirtualKey) => key.channel_mode === 'all' ? '全部渠道' : key.channel_ids.map(id => channels.data?.find(channel => channel.id === id)?.name ?? '').join(' ')
  const firstUsable = usableKeys(query.data)[0]
  return <>
    <PageHeader title="API Key" description="客户端使用 API Key 调用网关。每个 Key 可选择接入的渠道和模型，并按需添加 Token 与费用限制。" action={<div className="flex gap-2"><Button variant="outline" onClick={() => setGroupSheet({ group: null })}><FolderPlus />新建分组</Button><Button onClick={showCreate}><Plus />创建 API Key</Button></div>} />
    <GatewayAddress onGuide={firstUsable ? () => setGuide({ id: firstUsable.id, name: firstUsable.name }) : undefined} />
    {secret && <Secret key={secret} value={secret} title="完整 API Key，请妥善保存" onClose={() => setSecret('')} />}
    <ErrorBox error={copyError} />
    <ErrorBox error={query.error} retry={() => query.refetch()} />
    <ErrorBox error={toggle.error} />
    {query.isPending ? <Loading /> : !query.isError && <DataTable data={rows} toolbar={!!groups.data?.length && <>
      <div className="w-48 shrink-0"><Choice aria-label="按分组筛选" value={groupFilter || 'all'} onChange={value => setGroupFilter(value === 'all' ? '' : value)} options={[{ value: 'all', label: '全部分组' }, { value: 'none', label: '未分组' }, ...(groups.data ?? []).map(group => ({ value: group.id, label: `${group.name}（${group.key_count}）` }))]} /></div>
      {filterGroup && <Button variant="outline" size="sm" onClick={() => setGroupSheet({ group: filterGroup })}><Pencil />编辑分组</Button>}
    </>} columns={[
      { accessorKey: 'name', header: '名称', cell: ({ row }) => <div><div className="flex items-center gap-2"><span className="font-medium">{row.original.name}</span><KeyStatus keyItem={row.original} /></div>{row.original.holder && <div className="mt-0.5 text-xs text-muted-foreground">持有人：{row.original.holder}</div>}{(row.original.group_id || row.original.channel_count > 0 || row.original.share_expires_at) && <div className="mt-1 flex items-center gap-3 text-xs text-muted-foreground">{row.original.group_id && <button type="button" className="flex min-w-0 max-w-40 items-center gap-1 hover:text-foreground" title="渠道和模型跟随此分组，点击只看这个分组" onClick={() => setGroupFilter(row.original.group_id!)}><Folder aria-hidden="true" className="size-3.5 shrink-0" /><span className="truncate">{groupName(row.original)}</span></button>}<KeyChannelCount keyItem={row.original} /></div>}</div> },
      { id: 'scope', header: '渠道与模型', accessorFn: r => `${groupName(r)} ${channelNames(r)} ${r.allowed_models.join(' ') || '全部模型'}`, cell: ({ row }) => <KeyScope keyItem={row.original} channels={channels.data} /> },
      { id: 'tokens', header: () => <div className="text-right" title="按 UTC 自然日统计，北京时间 08:00 重置">今日 Token</div>, cell: ({ row }) => <Metric label="今日 Token" used={row.original.used_tokens_today} limit={row.original.token_limit_daily} format={tokens} reset={resetSince(row.original.usage_reset_daily_at, true, '今日用量')} /> },
      { id: 'today', header: () => <div className="text-right" title="按 UTC 自然日统计，北京时间 08:00 重置">今日费用</div>, cell: ({ row }) => <Metric label="今日费用" used={row.original.used_cost_today_usd} limit={row.original.cost_limit_daily_usd} format={money} reset={resetSince(row.original.usage_reset_daily_at, true, '今日用量')} /> },
      { id: 'total', header: () => <div className="text-right">累计费用</div>, cell: ({ row }) => <Metric label="累计费用" used={row.original.used_cost_total_usd} limit={row.original.cost_limit_total_usd} format={money} reset={resetSince(row.original.usage_reset_total_at, false, '累计费用')} /> },
      { id: 'rate', header: () => <div className="text-right">限速</div>, cell: ({ row }) => <dl className="ml-auto grid w-max grid-cols-[auto_3.25rem] items-baseline gap-x-2 gap-y-1.5 tabular-nums">
        <dt className="text-xs text-muted-foreground">请求</dt><dd className="text-right text-sm">{row.original.requests_per_minute != null ? <>{row.original.requests_per_minute.toLocaleString('zh-CN')}<span className="text-xs text-muted-foreground"> /分</span></> : <span className="text-muted-foreground">不限</span>}</dd>
        <dt className="text-xs text-muted-foreground">并发</dt><dd className="text-right text-sm">{row.original.concurrency_limit}</dd>
      </dl> },
      { id: 'actions', header: () => <span className="sr-only">操作</span>, cell: ({ row }) => <div className="flex items-center justify-end gap-0.5">
        {row.original.can_copy
          ? <Button type="button" size="icon-sm" variant="ghost" disabled={copyingId !== null} aria-label={`复制完整 API Key ${row.original.name}`} title={copyingId === row.original.id ? '复制中…' : copiedId === row.original.id ? '已复制完整 API Key' : `复制完整 API Key（${row.original.key_prefix}…）`} onClick={() => copyKey(row.original)}>{copiedId === row.original.id ? <Check className="size-4 text-success" /> : <Copy className="size-4" />}</Button>
          : <span className="inline-flex" title={`${row.original.key_prefix}… 创建时仅保存了哈希，无法复制；如需复制请重新创建`}><Button type="button" size="icon-sm" variant="ghost" disabled aria-label={`${row.original.name} 不可复制`}><Copy className="size-4" /></Button></span>}
        {!row.original.revoked_at && <>
          <Button size="icon-sm" variant="ghost" aria-label="编辑" title="编辑" onClick={() => showEdit(row.original)}><Pencil className="size-4" /></Button>
          <Button size="icon-sm" variant="ghost" aria-label={row.original.enabled ? '停用' : '启用'} title={row.original.enabled ? '停用' : '启用'} disabled={toggle.isPending} onClick={() => toggle.mutate(row.original)}><Power className={`size-4${row.original.enabled ? '' : ' text-success'}`} /></Button>
        </>}
        <DropdownMenu>
          <DropdownMenuTrigger asChild><Button size="icon-sm" variant="ghost" aria-label={`更多操作 ${row.original.name}`} title="更多操作"><MoreHorizontal className="size-4" /></Button></DropdownMenuTrigger>
          <DropdownMenuContent>
            {row.original.enabled && !row.original.revoked_at && <DropdownMenuItem onSelect={() => setGuide(row.original)}><Plug />接入说明</DropdownMenuItem>}
            <DropdownMenuItem asChild><Link to={`/usage?key=${row.original.id}`}><BarChart3 />查看用量</Link></DropdownMenuItem>
            {!row.original.revoked_at && <>
              <DropdownMenuItem onSelect={() => setQuota(row.original)}><History />额度记录</DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuItem onSelect={() => copyConfig(row.original)}><CopyPlus />模板创建 Key</DropdownMenuItem>
              {CLOUD && <DropdownMenuItem disabled={!row.original.enabled} title={row.original.enabled ? undefined : '请先启用这把 Key 再分享'} onSelect={() => setSharing(row.original)}><Link2 />分享为渠道</DropdownMenuItem>}
              <DropdownMenuSeparator />
              <DropdownMenuItem disabled={!usedToday(row.original)} title={usedToday(row.original) ? undefined : '今日暂无用量'} onSelect={() => askReset(row.original, 'today')}><TimerReset />重置今日用量</DropdownMenuItem>
              <DropdownMenuItem onSelect={() => askReset(row.original, 'all')}><Eraser />重置全部用量</DropdownMenuItem>
              <DropdownMenuItem variant="destructive" onSelect={() => { revoke.reset(); setRemove(row.original) }}><Trash2 />撤销</DropdownMenuItem>
            </>}
          </DropdownMenuContent>
        </DropdownMenu>
      </div> },
    ]} empty={query.data?.length
      ? <Empty title={filterGroup ? '此分组还没有 Key' : '没有符合条件的 Key'} description={filterGroup ? '在分组设置中勾选已有 Key，或创建新 Key。' : '切换分组查看其他 Key。'} action={filterGroup && <div className="flex gap-2"><Button variant="outline" onClick={() => setGroupSheet({ group: filterGroup })}>添加已有 Key</Button><Button variant="outline" onClick={showCreate}>创建 API Key</Button></div>} />
      : <Empty title="创建第一个 API Key" description="创建后可复制完整 API Key，请保存到应用的密钥管理系统。" action=<Button variant="outline" onClick={showCreate}>创建 API Key</Button> />} />}
    <DesktopPromo />
    <FormSheet open={open} onOpenChange={value => { if (!pending) setOpen(value) }} title={editing ? `编辑 API Key · ${editing.name}` : '创建 API Key'}>
      <form className="grid gap-4" onSubmit={form.handleSubmit(values => editing ? update.mutate({ key: editing, values }) : create.mutate(values))}>
        <div className="grid gap-2">
          <Field label="Key 名称" error={form.formState.errors.name?.message}>{id => <Input id={id} {...form.register('name')} placeholder="生产应用" />}</Field>
          <Field label="持有人（可选）" error={form.formState.errors.holder?.message}>{id => <Input id={id} {...form.register('holder')} placeholder="张三 · 研发部" />}</Field>
          {!editing && <p className="text-xs leading-5 text-muted-foreground">Key 前缀可在<Link to="/settings" className="text-primary hover:underline">帐号设置</Link>中自定义，只影响之后创建的 Key。</p>}
        </div>
        <Field label="分组" error={form.formState.errors.new_group?.message}>{id => <>
          <Choice id={id} aria-label="分组" value={groupId || 'none'} onChange={chooseGroup} options={[{ value: 'none', label: '不分组' }, ...(groups.data ?? []).map(group => ({ value: group.id, label: group.name })), { value: NEW_GROUP, label: '+ 新建分组' }]} />
          {groupId === NEW_GROUP && <Input aria-label="新分组名称" autoFocus placeholder="分组名称，如 学生班" {...form.register('new_group')} />}
        </>}</Field>
        {formGroup ? <GroupScopeSummary group={formGroup} channels={channels.data} />
          : <>
            {groupId === NEW_GROUP && <p className="-mt-2 text-xs text-muted-foreground">将用下方的渠道和模型创建分组。</p>}
            <ScopeFields value={form.watch()} onChange={setScope} errors={{ channel_ids: form.formState.errors.channel_ids?.message, allowed_models: form.formState.errors.allowed_models?.message }} channels={channels.data} />
          </>}
        <div className="grid grid-cols-2 gap-3">
          <Field label="每分钟请求数" error={form.formState.errors.requests_per_minute?.message}>{id => <Input id={id} type="number" min="1" placeholder="不限" {...form.register('requests_per_minute')} />}</Field>
          <Field label="并发上限" error={form.formState.errors.concurrency_limit?.message}>{id => <Input id={id} type="number" min="1" max="100" placeholder="1–100" {...form.register('concurrency_limit')} />}</Field>
        </div>
        <section className="grid gap-3 rounded-lg border border-border p-3" aria-label="用量限制">
          <div className="flex items-center justify-between gap-3">
            <p className="text-sm font-medium">用量限制</p>
            <DropdownMenu>
              <DropdownMenuTrigger asChild><Button type="button" variant="outline" size="sm" disabled={activeLimits.length === keyLimitFields.length}><Plus />添加限制</Button></DropdownMenuTrigger>
              <DropdownMenuContent>{keyLimitFields.filter(field => !activeLimits.includes(field)).map(field => <DropdownMenuItem key={field} onSelect={() => addLimit(field)}>{limitFields[field].menu}</DropdownMenuItem>)}</DropdownMenuContent>
            </DropdownMenu>
          </div>
          {activeLimits.length ? keyLimitFields.filter(field => activeLimits.includes(field)).map(field => <Field key={field} label={limitFields[field].label} help={limitFields[field].help} error={form.formState.errors[field]?.message} action={<Button type="button" variant="ghost" size="icon-xs" aria-label={`移除${limitFields[field].menu}限制`} title="移除此限制" onClick={() => removeLimit(field)}><X className="size-3.5" /></Button>}>{id => <>
            <Input id={id} type="number" min={limitFields[field].min} step="any" autoFocus={!form.getValues(field)} {...form.register(field)} />
            {editing && field === 'cost_limit_total_usd' && <TotalLimitIncrease keyItem={editing} limit={totalLimit} onIncreased={increased} />}
          </>}</Field>)
            : <p className="text-sm text-muted-foreground">未添加限制，Token 和费用不受限。</p>}
        </section>
        <ErrorBox error={editing ? update.error : create.error} />
        <div className="flex justify-end gap-2"><Button type="button" variant="outline" disabled={pending} onClick={() => setOpen(false)}>取消</Button><Button type="submit" disabled={pending}>{pending ? '保存中…' : editing ? '保存设置' : '创建 API Key'}</Button></div>
      </form>
    </FormSheet>
    {/* One sheet for every way into client setup; a new key adds the one-time save step on top. */}
    <FormSheet open={!!guide} onOpenChange={value => { if (!value) { setGuide(null); setCreated(null); create.reset() } }} title={guide ? `${guide.created ? 'API Key 已创建' : '接入客户端'} · ${guide.name}` : '接入客户端'} description={guide?.created ? '先保存完整 Key，再把配置复制到你使用的工具。' : '复制配置到你使用的工具，然后测试连接。'}>
      {guide && <ConnectGuide key={guide.id} keys={query.data ?? []} initialKeyId={guide.id} secret={created ?? undefined} showSecret={guide.created} />}
    </FormSheet>
    <GroupSheet open={!!groupSheet} group={groupSheet?.group ?? null} channels={channels.data} keys={query.data ?? []} groups={groups.data ?? []} onClose={() => setGroupSheet(null)} onSaved={id => { setGroupSheet(null); setGroupFilter(id) }} onDeleted={() => { setGroupSheet(null); setGroupFilter('') }} />
    {CLOUD && <ShareKeySheet keyItem={sharing} onClose={() => setSharing(null)} />}
    <QuotaSheet keyItem={quota && (query.data?.find(item => item.id === quota.id) ?? quota)} onClose={() => setQuota(null)} />
    <Confirm open={!!remove} onOpenChange={value => { if (!value && !revoke.isPending) setRemove(null) }} title="撤销 API Key？" description={`「${remove?.name ?? ''}」会立即停止接受请求。${remove?.channel_count ? `${remove.channel_count} 个帐号把它用作渠道，这些渠道也会立即失效。` : ''}${remove?.share_expires_at ? '待接受的分享链接会一并失效。' : ''}此操作不能撤回。`} pending={revoke.isPending} error={revoke.error} onConfirm={() => { if (remove) revoke.mutate(remove.id) }} />
    <Confirm open={!!resetting} onOpenChange={value => { if (!value && !resetUsage.isPending) setResetting(null) }} title={resetting ? resetCopy[resetting.scope].title : ''} description={resetting ? resetCopy[resetting.scope].description(resetting.key) : ''} pending={resetUsage.isPending} error={resetUsage.error} onConfirm={() => { if (resetting) resetUsage.mutate(resetting) }} />
  </>
}
