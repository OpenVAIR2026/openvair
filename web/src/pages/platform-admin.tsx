import { useDeferredValue, useState } from 'react'
import { useMutation, useQuery } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'
import { Activity, Database, RefreshCw, Users } from 'lucide-react'
import { api, formatDate, formatShortDate, jsonBody, type AdminAccount, type HealthCheck, type PublicVerifyCapacity, type RegistrationSettings, type SystemHealth } from '../lib/api'
import { queryClient, useUser } from '../lib/queries'
import { Badge, Choice, Confirm, DataTable, Empty, ErrorBox, Field, FormDialog, Loading, PageHeader, Panel, StatCard, StatGrid } from '../components/common'
import { Button } from '../components/ui/button'
import { Checkbox } from '../components/ui/checkbox'
import { Input } from '../components/ui/input'
import { SuperAdminGuard } from './platform'

const STATUS_OPTIONS = [{ value: 'all', label: '全部状态' }, { value: 'active', label: '正常' }, { value: 'disabled', label: '已停用' }]

function DisableDialog({ account, onClose }: { account: AdminAccount; onClose: () => void }) {
  const [reason, setReason] = useState('')
  const mutation = useMutation({
    mutationFn: () => api(`/platform/accounts/${account.id}/status`, jsonBody({ disabled: true, reason }, 'PATCH')),
    onSuccess: async () => { await queryClient.invalidateQueries({ queryKey: ['platform-accounts'] }); onClose() },
  })
  return <FormDialog open onOpenChange={open => { if (!open && !mutation.isPending) onClose() }} title={`停用 ${account.name}`} description={`${account.email} 会立即退出所有设备，无法登录，名下 API Key 停止鉴权。数据会保留，可随时恢复。`}>
    <form className="grid gap-4" onSubmit={event => { event.preventDefault(); mutation.mutate() }}>
      <Field label="停用原因" hint="选填，最多 200 个字符，只在系统管理和操作日志中显示。">{id => <Input id={id} maxLength={200} value={reason} disabled={mutation.isPending} onChange={event => setReason(event.target.value)} placeholder="例如：滥用接口" />}</Field>
      <ErrorBox error={mutation.error} />
      <div className="flex justify-end gap-2">
        <Button type="button" variant="outline" disabled={mutation.isPending} onClick={onClose}>取消</Button>
        <Button type="submit" variant="destructive" disabled={mutation.isPending}>{mutation.isPending ? '停用中…' : '停用帐号'}</Button>
      </div>
    </form>
  </FormDialog>
}

export function PlatformAccountsPage() {
  const user = useUser()
  const [search, setSearch] = useState('')
  const [status, setStatus] = useState('all')
  const query = useDeferredValue(search.trim())
  const [disabling, setDisabling] = useState<AdminAccount | null>(null)
  const [enabling, setEnabling] = useState<AdminAccount | null>(null)
  const accounts = useQuery({
    queryKey: ['platform-accounts', query, status],
    queryFn: ({ signal }) => api<{ accounts: AdminAccount[]; total: number }>(`/platform/accounts?${new URLSearchParams({ q: query, status })}`, { signal }),
    placeholderData: previous => previous,
  })
  const enable = useMutation({
    mutationFn: (account: AdminAccount) => api(`/platform/accounts/${account.id}/status`, jsonBody({ disabled: false }, 'PATCH')),
    onSuccess: async () => { await queryClient.invalidateQueries({ queryKey: ['platform-accounts'] }); setEnabling(null) },
  })
  const columns: ColumnDef<AdminAccount, any>[] = [
    { accessorKey: 'name', header: '帐号', cell: ({ row }) => <div className="grid gap-0.5"><span className="font-medium">{row.original.name}</span><span className="text-xs text-muted-foreground">{row.original.email}</span></div> },
    { id: 'status', header: '状态', accessorFn: row => row.disabled_at ? '已停用' : '正常', cell: ({ row }) => <div className="flex flex-wrap gap-1.5">{row.original.disabled_at ? <Badge tone="danger">已停用</Badge> : <Badge tone="success">正常</Badge>}{row.original.is_super_admin && <Badge>超级管理员</Badge>}</div> },
    { accessorKey: 'channel_count', header: '渠道', cell: info => <span className="tabular-nums">{info.getValue<number>()}</span> },
    { accessorKey: 'key_count', header: 'API Key', cell: info => <span className="tabular-nums">{info.getValue<number>()}</span> },
    { accessorKey: 'requests_30d', header: '30 天请求', cell: info => <span className="tabular-nums">{info.getValue<number>().toLocaleString('zh-CN')}</span> },
    { accessorKey: 'last_active_at', header: '最近活跃', cell: info => { const value = info.getValue<string | null>(); return value ? formatShortDate(value) : <span className="text-muted-foreground">—</span> } },
    { accessorKey: 'created_at', header: '注册时间', cell: info => formatShortDate(info.getValue<string>()) },
    {
      id: 'actions', header: '操作', enableSorting: false, cell: ({ row }) => {
        const account = row.original
        if (account.disabled_at) return <div className="grid justify-items-start gap-1"><Button size="sm" variant="outline" onClick={() => { enable.reset(); setEnabling(account) }}>恢复</Button>{account.disabled_reason && <span className="max-w-48 truncate text-xs text-muted-foreground" title={account.disabled_reason}>{account.disabled_reason}</span>}</div>
        const locked = account.is_super_admin || account.id === user.account_id
        return <Button size="sm" variant="outline" disabled={locked} title={locked ? '不能停用超级管理员或自己的帐号' : undefined} onClick={() => setDisabling(account)}>停用</Button>
      },
    },
  ]
  const data = accounts.data
  return <SuperAdminGuard>
    <PageHeader title="帐号管理" description="查看所有帐号的资源和活跃情况。停用后用户无法登录，名下 API Key 立即失效，数据保留。" />
    <div className="mb-4 flex flex-wrap items-center gap-3">
      <Input aria-label="按邮箱或帐号名称搜索" className="max-w-xs" placeholder="按邮箱或帐号名称搜索" value={search} onChange={event => setSearch(event.target.value)} />
      <div className="w-36"><Choice aria-label="帐号状态" value={status} onChange={setStatus} options={STATUS_OPTIONS} /></div>
      {data && <span className="ml-auto text-xs text-muted-foreground">{data.total > data.accounts.length ? `共 ${data.total} 个，显示最近注册的 ${data.accounts.length} 个，可搜索缩小范围` : `${data.total} 个帐号`}</span>}
    </div>
    <ErrorBox error={accounts.error} retry={() => accounts.refetch()} />
    {accounts.isPending ? <Loading /> : data ? <DataTable searchable={false} data={data.accounts} columns={columns} empty={<Empty title="没有匹配的帐号" description="换个关键词或状态再试。" />} /> : null}
    {disabling && <DisableDialog account={disabling} onClose={() => setDisabling(null)} />}
    <Confirm open={!!enabling} onOpenChange={open => { if (!open) setEnabling(null) }} title={`恢复 ${enabling?.name ?? ''}`} description="用户可以重新登录，原有 API Key 恢复鉴权（已停用或已撤销的 Key 保持原状）。" onConfirm={() => enabling && enable.mutate(enabling)} pending={enable.isPending} error={enable.error} />
  </SuperAdminGuard>
}

function RegistrationForm({ current }: { current: RegistrationSettings }) {
  const [enabled, setEnabled] = useState(current.enabled)
  const [domains, setDomains] = useState(current.allowed_domains.join('\n'))
  const [verify, setVerify] = useState(current.require_email_verification)
  const [saved, setSaved] = useState(false)
  const mutation = useMutation({
    mutationFn: () => api<RegistrationSettings>('/platform/registration', jsonBody({ enabled, allowed_domains: domains.split(/[\s,，]+/).filter(Boolean), require_email_verification: verify }, 'PATCH')),
    onSuccess: result => { queryClient.setQueryData(['platform-registration'], result); queryClient.invalidateQueries({ queryKey: ['registration-status'] }); setDomains(result.allowed_domains.join('\n')); setSaved(true) },
  })
  return <Panel className="max-w-[840px] p-6">
    <form className="grid max-w-xl gap-5" onSubmit={event => { event.preventDefault(); setSaved(false); mutation.mutate() }}>
      <label className="flex items-start gap-3 text-sm">
        <Checkbox className="mt-0.5" checked={enabled} onCheckedChange={value => { setEnabled(value === true); setSaved(false) }} />
        <span className="grid gap-1"><span className="font-medium">允许新用户注册</span><span className="text-xs leading-5 text-muted-foreground">关闭后登录页不再显示注册入口。环境变量中配置的超级管理员邮箱不受限制。</span></span>
      </label>
      <label className="flex items-start gap-3 text-sm">
        <Checkbox className="mt-0.5" disabled={!enabled} checked={verify} onCheckedChange={value => { setVerify(value === true); setSaved(false) }} />
        <span className="grid gap-1"><span className="font-medium">注册时验证邮箱</span><span className="text-xs leading-5 text-muted-foreground">新用户需填写发送到注册邮箱的验证码才能创建帐号，避免冒用他人邮箱。</span>{verify && !current.smtp_configured && <span className="text-xs leading-5 text-warning">尚未配置 SMTP，验证暂不生效；在“邮箱设置”中配置后自动启用。</span>}</span>
      </label>
      <Field label="允许注册的邮箱域名" hint="每行一个，例如 example.com。留空表示不限制域名，最多 50 个。只匹配完整域名，不含子域名。">
        {id => <textarea id={id} rows={5} disabled={!enabled} value={domains} onChange={event => { setDomains(event.target.value); setSaved(false) }} placeholder="example.com" className="min-h-28 w-full rounded-md border border-input bg-transparent px-3 py-2 font-mono text-sm shadow-xs outline-none focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:opacity-50 dark:bg-input/30" />}
      </Field>
      <ErrorBox error={mutation.error} />
      <div className="flex flex-wrap items-center gap-3">
        <Button type="submit" disabled={mutation.isPending}>{mutation.isPending ? '保存中…' : '保存设置'}</Button>
        <span role="status" aria-live="polite" className="text-sm text-success">{saved ? '已保存' : ''}</span>
      </div>
    </form>
  </Panel>
}

export function PlatformRegistrationPage() {
  const settings = useQuery({ queryKey: ['platform-registration'], queryFn: ({ signal }) => api<RegistrationSettings>('/platform/registration', { signal }) })
  return <SuperAdminGuard>
    <PageHeader title="注册设置" description="控制是否开放注册、是否验证邮箱，以及哪些邮箱域名可以注册。已有帐号不受影响。" />
    <ErrorBox error={settings.error} retry={() => settings.refetch()} />
    {settings.isPending ? <Loading /> : settings.data ? <RegistrationForm current={settings.data} /> : null}
  </SuperAdminGuard>
}

const CAPACITY_FIELDS = [
  { key: 'workers', label: '同时检测数', min: 1, max: 32, hint: '全站同时运行的检测数，首页显示为“同时检测”。超出的检测排队等待。' },
  { key: 'concurrency', label: '单次检测并发请求', min: 1, max: 4, hint: '每个检测同时向被测上游发出的请求数。调高会更快出结果，但更容易触发对方限流。' },
  { key: 'max_waiting', label: '排队上限', min: 1, max: 500, hint: '运行中和排队中的检测合计达到这个数后，新提交的检测会被拒绝。不能小于同时检测数。' },
] as const

function CapacityForm({ current }: { current: PublicVerifyCapacity }) {
  const [values, setValues] = useState<Record<keyof PublicVerifyCapacity, string>>({ workers: String(current.workers), concurrency: String(current.concurrency), max_waiting: String(current.max_waiting) })
  const [saved, setSaved] = useState(false)
  const mutation = useMutation({
    mutationFn: () => api<PublicVerifyCapacity>('/platform/public-verify', jsonBody({ workers: Number(values.workers), concurrency: Number(values.concurrency), max_waiting: Number(values.max_waiting) }, 'PATCH')),
    onSuccess: result => { queryClient.setQueryData(['platform-public-verify'], result); setSaved(true) },
  })
  return <Panel className="max-w-[840px] p-6">
    <form className="grid max-w-xl gap-5" onSubmit={event => { event.preventDefault(); setSaved(false); mutation.mutate() }}>
      {CAPACITY_FIELDS.map(field => <Field key={field.key} label={field.label} hint={`${field.hint}范围 ${field.min}–${field.max}。`}>
        {id => <Input id={id} type="number" inputMode="numeric" required min={field.min} max={field.max} step={1} value={values[field.key]} onChange={event => { setValues(prev => ({ ...prev, [field.key]: event.target.value })); setSaved(false) }} />}
      </Field>)}
      <ErrorBox error={mutation.error} />
      <div className="flex flex-wrap items-center gap-3">
        <Button type="submit" disabled={mutation.isPending}>{mutation.isPending ? '保存中…' : '保存设置'}</Button>
        <span role="status" aria-live="polite" className="text-sm text-success">{saved ? '已保存' : ''}</span>
      </div>
    </form>
  </Panel>
}

function PublicUrlForm({ current }: { current: string | null }) {
  const [url, setUrl] = useState(current ?? '')
  const [confirming, setConfirming] = useState(false)
  const [saved, setSaved] = useState(false)
  const browser = window.location.origin
  const mutation = useMutation({
    mutationFn: () => api<{ url: string | null }>('/platform/public-url', jsonBody({ url: url.trim() || null }, 'PATCH')),
    onSuccess: result => { queryClient.setQueryData(['platform-public-url'], result); queryClient.invalidateQueries({ queryKey: ['server'] }); setUrl(result.url ?? ''); setConfirming(false); setSaved(true) },
  })
  // Saving an address other than the one in use is the step most likely to point everyone at the wrong place.
  const elsewhere = (value: string) => { const trimmed = value.trim().replace(/\/+$/, ''); return !!trimmed && trimmed.toLowerCase() !== browser.toLowerCase() && trimmed !== (current ?? '') }
  return <Panel className="max-w-[840px] p-6">
    <form className="grid max-w-xl gap-5" onSubmit={event => { event.preventDefault(); setSaved(false); if (elsewhere(url) && !confirming) { setConfirming(true); return } mutation.mutate() }}>
      <Field label="对外访问地址" hint={`同事访问这台服务器用的地址，只填协议、域名或 IP 和端口，例如 https://ai.example.com。留空时使用浏览器当前打开的地址，现在是 ${browser}。`}>
        {id => <Input id={id} value={url} inputMode="url" spellCheck={false} placeholder={browser} onChange={event => { setUrl(event.target.value); setConfirming(false); setSaved(false) }} className="font-mono" />}
      </Field>
      {confirming && <p role="alert" className="text-sm leading-6 text-warning">这个地址和你现在打开控制台用的地址不同。请先确认它已经能访问，保存后接入说明会改用这个地址，桌面客户端也会提示用户更换过去。</p>}
      <ErrorBox error={mutation.error} />
      <div className="flex flex-wrap items-center gap-3">
        <Button type="submit" disabled={mutation.isPending}>{mutation.isPending ? '保存中…' : confirming ? '确认保存' : '保存设置'}</Button>
        <span role="status" aria-live="polite" className="text-sm text-success">{saved ? '已保存' : ''}</span>
      </div>
    </form>
  </Panel>
}

export function PlatformPublicUrlPage() {
  const settings = useQuery({ queryKey: ['platform-public-url'], queryFn: ({ signal }) => api<{ url: string | null }>('/platform/public-url', { signal }) })
  return <SuperAdminGuard>
    <PageHeader title="访问地址" description="接入说明、网关地址和文档示例都使用这个地址。更换域名或 IP 时在这里改，域名解析、证书和反向代理需要另外配置。" />
    <ErrorBox error={settings.error} retry={() => settings.refetch()} />
    {settings.isPending ? <Loading /> : settings.data ? <PublicUrlForm current={settings.data.url} /> : null}
  </SuperAdminGuard>
}

export function PlatformPublicVerifyPage() {
  const settings = useQuery({ queryKey: ['platform-public-verify'], queryFn: ({ signal }) => api<PublicVerifyCapacity>('/platform/public-verify', { signal }) })
  return <SuperAdminGuard>
    <PageHeader title="模型验证" description="控制首页模型验证的并发和排队。保存后几秒内生效，正在运行的检测不会被中断。" />
    <ErrorBox error={settings.error} retry={() => settings.refetch()} />
    {settings.isPending ? <Loading /> : settings.data ? <CapacityForm current={settings.data} /> : null}
  </SuperAdminGuard>
}

const LEVEL_TONE = { ok: 'success', warn: 'warning', error: 'danger' } as const
const LEVEL_LABEL = { ok: '正常', warn: '注意', error: '异常' } as const

function CheckRow({ check }: { check: HealthCheck }) {
  return <div className="flex flex-wrap items-center gap-3 border-t border-border px-5 py-4 first:border-t-0">
    <span className="w-28 font-medium">{check.name}</span>
    <Badge tone={LEVEL_TONE[check.level]}><span className="size-1.5 rounded-full bg-current" />{LEVEL_LABEL[check.level]}</Badge>
    <span className="flex-1 text-sm text-muted-foreground">{check.detail}</span>
    {check.latency_ms != null && <span className="text-xs tabular-nums text-muted-foreground">{check.latency_ms} ms</span>}
  </div>
}

export function PlatformHealthPage() {
  const health = useQuery({ queryKey: ['platform-health'], queryFn: ({ signal }) => api<SystemHealth>('/platform/health', { signal }), refetchInterval: 30_000 })
  const data = health.data
  const stats = data?.stats
  return <SuperAdminGuard>
    <PageHeader title="系统状态" description="数据库、Redis、后台任务和邮件服务的运行状态，每 30 秒自动刷新。" action={<Button variant="outline" disabled={health.isFetching} onClick={() => health.refetch()}><RefreshCw aria-hidden="true" className={`size-4${health.isFetching ? ' animate-spin' : ''}`} />刷新</Button>} />
    <ErrorBox error={health.error} retry={() => health.refetch()} />
    {health.isPending ? <Loading /> : data ? <div className="grid gap-6">
      {stats && <StatGrid>
        <StatCard label="帐号" value={stats.accounts.toLocaleString('zh-CN')} detail={stats.disabled_accounts ? `${stats.disabled_accounts} 个已停用` : '无停用帐号'} icon={<Users aria-hidden="true" />} />
        <StatCard label="有效会话" value={stats.active_sessions.toLocaleString('zh-CN')} detail="未过期的登录" icon={<Activity aria-hidden="true" />} tone="success" />
        <StatCard label="24 小时请求" value={stats.requests_24h.toLocaleString('zh-CN')} detail={`${stats.errors_24h.toLocaleString('zh-CN')} 个失败`} icon={<Database aria-hidden="true" />} tone={stats.requests_24h && stats.errors_24h / stats.requests_24h > 0.2 ? 'warning' : 'primary'} />
        <StatCard label="质量检测排队" value={stats.quality_jobs_queued.toLocaleString('zh-CN')} detail="等待执行的任务" icon={<RefreshCw aria-hidden="true" />} tone={stats.quality_jobs_queued > 50 ? 'warning' : 'primary'} />
      </StatGrid>}
      <Panel>
        <div className="flex flex-wrap items-center gap-3 border-b border-border px-5 py-4"><h2 className="font-medium">服务检查</h2><Badge tone={LEVEL_TONE[data.status]}>{data.status === 'ok' ? '全部正常' : data.status === 'warn' ? '需要注意' : '存在异常'}</Badge><span className="ml-auto text-xs text-muted-foreground">检查于 {formatShortDate(data.checked_at)}</span></div>
        {data.checks.map(check => <CheckRow key={check.key} check={check} />)}
      </Panel>
      <Panel className="grid gap-3 p-5 text-sm sm:grid-cols-3">
        <div><p className="text-xs text-muted-foreground">版本</p><p className="mt-1 font-mono">{data.version}</p></div>
        <div><p className="text-xs text-muted-foreground">本次启动</p><p className="mt-1">{formatDate(data.started_at)}</p></div>
        <div><p className="text-xs text-muted-foreground">数据库迁移</p><p className="mt-1 font-mono">{data.migration ? `${data.migration.version} · ${data.migration.description}` : '—'}</p></div>
      </Panel>
    </div> : null}
  </SuperAdminGuard>
}
