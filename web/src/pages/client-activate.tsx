import { useState, type FormEvent } from 'react'
import { useMutation, useQuery } from '@tanstack/react-query'
import { Link, Navigate, useLocation, useSearchParams } from 'react-router-dom'
import { Check, CircleCheck, TriangleAlert, X } from 'lucide-react'
import { api, ApiError, formatShortDate, type User } from '../lib/api'
import { Button } from '../components/ui/button'
import { Input } from '../components/ui/input'
import { ErrorBox, Field, Loading, Panel } from '../components/common'
import { BrandMark, BrandName } from '../components/brand'
import { PRODUCT } from '../lib/edition'

interface DeviceRequest {
  client_name: string
  user_code: string
  status: 'pending' | 'approved' | 'denied' | 'consumed'
  created_at: string
  expires_at: string
}

/** Approves a VAIR Desktop sign-in: the app shows a code, the user confirms it here. */
export function ClientActivatePage() {
  const location = useLocation()
  const [params, setParams] = useSearchParams()
  const code = params.get('code')?.trim() ?? ''
  const me = useQuery({ queryKey: ['me'], queryFn: ({ signal }) => api<User>('/me', { signal }), retry: false })

  if (me.isPending) return <Frame><Loading /></Frame>
  if (me.isError) return me.error instanceof ApiError && me.error.status === 401
    ? <Navigate to="/login" state={{ from: `${location.pathname}${location.search}` }} replace />
    : <Frame><ErrorBox error={me.error} retry={() => me.refetch()} /></Frame>
  return <Frame>{code ? <Request code={code} email={me.data.email} /> : <EnterCode onSubmit={value => setParams({ code: value })} />}</Frame>
}

function Frame({ children }: { children: React.ReactNode }) {
  return <main className="auth-page grid min-h-dvh place-items-center p-4"><div className="w-full max-w-lg">
    <Link to="/" className="brand auth-brand mb-7 flex justify-center" aria-label={`${PRODUCT} 首页`}><BrandMark /><BrandName /></Link>
    <Panel className="p-6 sm:p-8">{children}</Panel>
  </div></main>
}

function EnterCode({ onSubmit }: { onSubmit: (code: string) => void }) {
  const [value, setValue] = useState('')
  const submit = (event: FormEvent) => { event.preventDefault(); if (value.trim()) onSubmit(value.trim()) }
  return <form className="grid gap-5" onSubmit={submit}>
    <div>
      <h1 className="text-[24px] font-semibold tracking-tight">授权 VAIR Desktop</h1>
      <p className="mt-2 text-sm leading-6 text-muted-foreground">输入 VAIR Desktop 中显示的 8 位授权码。</p>
    </div>
    <Field label="授权码">{id => <Input id={id} value={value} onChange={e => setValue(e.target.value)} autoComplete="off" autoCapitalize="characters" spellCheck={false} placeholder="KQ7M-3XPD" className="font-mono tracking-widest" />}</Field>
    <Button type="submit" className="h-11">继续</Button>
  </form>
}

function Request({ code, email }: { code: string; email: string }) {
  const request = useQuery({
    queryKey: ['client-device', code],
    queryFn: ({ signal }) => api<DeviceRequest>(`/client/device/${encodeURIComponent(code)}`, { signal }),
    retry: false,
  })
  const decide = useMutation({
    mutationFn: (decision: 'approve' | 'deny') => api(`/client/device/${encodeURIComponent(code)}/${decision}`, { method: 'POST' }).then(() => decision),
  })

  if (request.isPending) return <Loading />
  if (request.isError) return request.error instanceof ApiError && request.error.status === 404
    ? <Outcome tone="warning" title="授权码无效或已过期" description="请回到 VAIR Desktop 重新登录，获取新的授权码。" />
    : <ErrorBox error={request.error} retry={() => request.refetch()} />
  if (decide.data === 'approve' || request.data.status === 'approved' || request.data.status === 'consumed')
    return <Outcome tone="success" title="已授权" description={`${request.data.client_name} 已登录。回到 VAIR Desktop 继续配置 Agent。`} />
  if (decide.data === 'deny' || request.data.status === 'denied')
    return <Outcome tone="warning" title="已拒绝" description="这台设备不会获得访问权限。" />

  const device = request.data
  return <div className="grid gap-5">
    <div>
      <h1 className="text-[24px] font-semibold tracking-tight">授权 VAIR Desktop</h1>
      <p className="mt-2 text-sm text-muted-foreground">以 {email} 登录</p>
    </div>
    <dl className="grid gap-3 sm:grid-cols-2">
      <div className="rounded-lg border border-border bg-background/40 p-3"><dt className="text-xs text-muted-foreground">设备</dt><dd className="mt-1 text-sm">{device.client_name}</dd><dd className="mt-1 text-xs text-muted-foreground">{formatShortDate(device.created_at)} 发起</dd></div>
      <div className="rounded-lg border border-primary/30 bg-background/40 p-3"><dt className="text-xs text-muted-foreground">授权码</dt><dd className="mt-1 font-mono text-lg tracking-widest text-primary">{device.user_code}</dd><dd className="mt-1 text-xs text-muted-foreground">应与客户端显示一致</dd></div>
    </dl>
    <div className="grid gap-4 sm:grid-cols-2">
      <ul className="grid gap-2 text-sm" aria-label="可以">
        <li className="text-xs font-medium text-success">可以</li>
        <li className="flex gap-2"><Check aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-success" />查看你的 Key 和可用模型</li>
        <li className="flex gap-2"><Check aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-success" />读取所选 Key，写入本机 Agent</li>
      </ul>
      <ul className="grid gap-2 text-sm text-muted-foreground" aria-label="不可以">
        <li className="text-xs font-medium text-destructive">不可以</li>
        <li className="flex gap-2"><X aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-destructive" />创建、修改、停用或撤销 Key</li>
        <li className="flex gap-2"><X aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-destructive" />访问渠道和上游供应商凭据</li>
      </ul>
    </div>
    <p className="flex items-start gap-2 rounded-lg border border-warning/30 bg-warning/5 p-3 text-sm text-warning"><TriangleAlert aria-hidden="true" className="mt-0.5 size-4 shrink-0" />只有在你本人刚刚从 VAIR Desktop 发起登录时才授权。</p>
    <ErrorBox error={decide.error} />
    <div className="flex flex-wrap items-center gap-3">
      <span className="flex-1 text-xs text-muted-foreground">30 天有效，可在帐号设置中随时吊销</span>
      <Button variant="outline" disabled={decide.isPending} onClick={() => decide.mutate('deny')}>拒绝</Button>
      <Button disabled={decide.isPending} onClick={() => decide.mutate('approve')}>{decide.isPending ? '处理中…' : '授权此设备'}</Button>
    </div>
  </div>
}

function Outcome({ tone, title, description }: { tone: 'success' | 'warning'; title: string; description: string }) {
  const Icon = tone === 'success' ? CircleCheck : TriangleAlert
  return <div className="grid justify-items-center gap-3 py-4 text-center" role="status">
    <Icon aria-hidden="true" className={tone === 'success' ? 'size-10 text-success' : 'size-10 text-warning'} />
    <h1 className="text-xl font-semibold">{title}</h1>
    <p className="max-w-sm text-sm leading-6 text-muted-foreground">{description}</p>
  </div>
}
