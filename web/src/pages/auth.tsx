import { useEffect, useState, type FormEvent } from 'react'
import { useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import { z } from 'zod'
import { useMutation, useQuery } from '@tanstack/react-query'
import { Link, Navigate, useLocation, useNavigate } from 'react-router-dom'
import { ArrowRight } from 'lucide-react'
import { api, ApiError, jsonBody, type User } from '../lib/api'
import { queryClient } from '../lib/queries'
import { Button } from '../components/ui/button'
import { Input } from '../components/ui/input'
import { ErrorBox, Field, Loading, Panel } from '../components/common'
import { BrandMark, BrandName } from '../components/brand'
import { CLOUD, PRODUCT } from '../lib/edition'

type Mode = 'login' | 'register' | 'reset'

function cookieMayBeDropped() {
  const host = window.location.hostname
  return window.location.protocol === 'http:' && !(host === 'localhost' || host.endsWith('.localhost') || host === '127.0.0.1' || host === '[::1]')
}

export function AuthPage() {
  const [mode, setMode] = useState<Mode>('login')
  const [notice, setNotice] = useState('')
  const registration = useQuery({ queryKey: ['registration-status'], queryFn: ({ signal }) => api<{ enabled: boolean; domain_restricted: boolean; email_verification?: boolean }>('/auth/registration', { signal }), retry: false, staleTime: 60_000, enabled: CLOUD })
  // A fresh OpenVAIR instance has no one to sign in as until setup creates the administrator.
  const setup = useSetupStatus()
  const registrationClosed = registration.data?.enabled === false
  const verifyEmail = mode === 'register' && registration.data?.email_verification === true
  const schema = z.object({ email: z.string().email('请输入有效邮箱'), password: z.string().min(10, '密码至少 10 个字符').max(256, '密码最长 256 个字符'), account_name: z.string(), code: z.string() }).superRefine((v, ctx) => {
    if (mode === 'register' && !v.account_name.trim()) ctx.addIssue({ code: 'custom', path: ['account_name'], message: '请输入帐号名称' })
    if (verifyEmail && !/^\d{6}$/.test(v.code)) ctx.addIssue({ code: 'custom', path: ['code'], message: '请输入 6 位验证码' })
  })
  const form = useForm<z.infer<typeof schema>>({ resolver: zodResolver(schema), defaultValues: { email: '', password: '', account_name: '', code: '' } })
  const signup = useSignupCode(form.watch('email'))
  const navigate = useNavigate()
  const next = returnPath(useLocation().state)
  const me = useQuery({ queryKey: ['me'], queryFn: ({ signal }) => api<User>('/me', { signal }), retry: false })
  const mutation = useMutation({
    mutationFn: async ({ code, ...values }: z.infer<typeof schema>) => {
      if (verifyEmail && !signup.requestId) throw new Error('请先获取邮箱验证码')
      const body = mode === 'register' ? { ...values, ...(verifyEmail && { request_id: signup.requestId, code }) } : { email: values.email, password: values.password }
      try {
        const result = await api(`/auth/${mode}`, jsonBody(body))
        if (mode === 'login' && cookieMayBeDropped()) {
          // A Secure cookie is never stored over plain HTTP, so the login "succeeds" but no session sticks.
          try { await api<User>('/me') } catch (error) {
            if (error instanceof ApiError && error.status === 401) throw new Error('登录成功，但浏览器没有保存登录状态：通过 http 访问非本机地址时需要 HTTPS。请改用 HTTPS 访问，或在服务器 .env 中设置 COOKIE_SECURE=false 后重启。')
            throw error
          }
        }
        return result
      } catch (error) {
        // Sign-in failures never say whether the email is registered.
        if (mode === 'login' && error instanceof ApiError && error.status === 401) throw new Error('邮箱或密码不正确')
        throw error
      }
    },
    onSuccess: async () => { await queryClient.cancelQueries(); queryClient.clear(); navigate(next, { replace: true }) },
  })
  const switchMode = (next: Mode) => { setMode(next); setNotice(''); mutation.reset(); signup.reset(); form.setValue('code', ''); form.clearErrors() }
  const sendCode = async () => { if (await form.trigger('email')) signup.send(form.getValues('email')) }
  if (me.isPending || setup.isLoading) return <div className="mx-auto max-w-4xl p-8"><Loading /></div>
  if (me.isSuccess && !mutation.isPending) return <Navigate to={next} replace />
  if (setup.data?.needed) return <Navigate to="/setup" replace />
  const title = mode === 'login' ? '登录控制台' : mode === 'register' ? '创建你的帐号' : '重置密码'

  return <main className="auth-page grid min-h-dvh place-items-center p-4"><div className="w-full max-w-md">
    <Link to="/" className="brand auth-brand mb-7 flex justify-center" aria-label={`${PRODUCT} 首页`}><BrandMark /><BrandName /></Link>
    <Panel className="p-6 sm:p-8">
      <span className="text-xs font-medium tracking-wider text-primary">BYOK GATEWAY</span>
      <h1 className="mt-4 text-[28px] font-semibold tracking-tight">{title}</h1>
      <p className="mt-2 text-sm leading-6 text-muted-foreground">{mode === 'reset' ? '验证码会发送到你的登录邮箱。' : CLOUD && next.startsWith('/share') ? '登录或注册后即可把分享给你的 Key 添加为渠道。' : '多家 AI 渠道，一个入口。'}</p>
      {mode === 'reset'
        ? <ResetPassword initialEmail={form.getValues('email')} onDone={email => { switchMode('login'); form.setValue('email', email); form.setValue('password', ''); setNotice('密码已重置，请使用新密码登录。') }} />
        : <form className="mt-7 grid gap-4" onSubmit={form.handleSubmit(v => mutation.mutate(v))}>
          {notice && <p role="status" className="text-sm text-success">{notice}</p>}
          <Field label="邮箱" error={form.formState.errors.email?.message}>{id => verifyEmail
            ? <div className="flex gap-2"><Input id={id} type="email" autoComplete="email" className="min-w-0 flex-1" {...form.register('email')} placeholder="you@company.com" /><Button type="button" variant="outline" className="shrink-0" disabled={signup.sending || signup.cooldown > 0} onClick={sendCode}>{signup.sending ? '发送中…' : signup.cooldown > 0 ? `重新发送 (${signup.cooldown}s)` : signup.requestId ? '重新发送' : '发送验证码'}</Button></div>
            : <Input id={id} type="email" autoComplete="email" {...form.register('email')} placeholder="you@company.com" />}</Field>
          {verifyEmail && signup.sentTo && <p role="status" className="-mt-2 text-xs leading-5 text-success">验证码已发送到 {signup.sentTo}，10 分钟内有效。如果该邮箱已注册，你会收到一封登录提醒。</p>}
          {verifyEmail && <ErrorBox error={signup.error} />}
          {mode === 'register' && registration.data?.domain_restricted && <p className="text-xs leading-5 text-muted-foreground">当前仅允许指定邮箱域名注册。</p>}
          {verifyEmail && <Field label="邮箱验证码" error={form.formState.errors.code?.message}>{id => <Input id={id} inputMode="numeric" autoComplete="one-time-code" {...form.register('code', { onChange: event => form.setValue('code', event.target.value.replace(/\D/g, '').slice(0, 6), { shouldValidate: !!form.formState.errors.code }) })} placeholder="6 位验证码" />}</Field>}
          {mode === 'register' && <Field label="帐号名称" error={form.formState.errors.account_name?.message}>{id => <Input id={id} autoComplete="off" {...form.register('account_name')} maxLength={120} placeholder="Acme AI" />}</Field>}
          <Field label="密码" error={form.formState.errors.password?.message} action={mode === 'login' && <button type="button" className="text-xs text-muted-foreground underline-offset-4 hover:text-foreground hover:underline" onClick={() => switchMode('reset')}>忘记密码？</button>}>{id => <Input id={id} type="password" autoComplete={mode === 'login' ? 'current-password' : 'new-password'} {...form.register('password')} />}</Field>
          <ErrorBox error={mutation.error} />
          <Button className="mt-2 h-11" disabled={mutation.isPending} type="submit">{mutation.isPending ? '处理中…' : mode === 'login' ? '登录' : '创建帐号'}<ArrowRight aria-hidden="true" className="size-4" /></Button>
        </form>}
      {!CLOUD ? null : mode === 'login' && registrationClosed
        ? <p className="mt-4 text-center text-sm text-muted-foreground">暂未开放注册，如需帐号请联系管理员。</p>
        : <Button className="mt-4 w-full text-muted-foreground" variant="ghost" onClick={() => switchMode(mode === 'login' ? 'register' : 'login')}>{mode === 'login' ? '还没有帐号？创建帐号' : '已有帐号？返回登录'}</Button>}
    </Panel>
    {mode === 'login' && <p className="mt-4 text-center text-sm text-muted-foreground">只有 {PRODUCT} Key？<Link to="/my-key" className="text-primary hover:underline">查看额度与接入方式</Link></p>}
    <p className="mt-6 text-center text-xs text-muted-foreground">上游凭证加密存储 · 请求正文默认不留存</p>
  </div></main>
}

/** Requests a sign-up code; a code only counts for the address it was sent to. */
function useSignupCode(currentEmail: string) {
  const [requestId, setRequestId] = useState('')
  const [sentTo, setSentTo] = useState('')
  const [cooldown, setCooldown] = useState(0)
  const request = useMutation({
    mutationFn: async (email: string) => {
      try {
        return await api<{ request_id: string; retry_after: number }>('/auth/register/code', jsonBody({ email }))
      } catch (error) {
        if (error instanceof ApiError && error.status === 429) throw new Error('发送太频繁，请稍后再试')
        throw error
      }
    },
    onSuccess: (result, email) => { setRequestId(result.request_id); setSentTo(email); setCooldown(result.retry_after) },
  })
  useEffect(() => {
    if (cooldown <= 0) return
    const timer = window.setTimeout(() => setCooldown(value => value - 1), 1000)
    return () => window.clearTimeout(timer)
  }, [cooldown])
  // Editing the email abandons the code sent to the previous address.
  const current = currentEmail.trim().toLowerCase()
  const stale = !!sentTo && current !== sentTo
  useEffect(() => { if (stale) { setRequestId(''); setSentTo(''); setCooldown(0) } }, [stale])
  return {
    requestId: stale ? '' : requestId,
    sentTo: stale ? '' : sentTo,
    cooldown: stale ? 0 : cooldown,
    sending: request.isPending,
    error: request.error,
    send: (email: string) => request.mutate(email.trim().toLowerCase()),
    reset: () => { request.reset(); setRequestId(''); setSentTo(''); setCooldown(0) },
  }
}

/** Whether the instance still needs its administrator; always false on the hosted service. */
export function useSetupStatus() {
  return useQuery({ queryKey: ['setup'], queryFn: ({ signal }) => api<{ needed: boolean }>('/setup', { signal }), retry: false, enabled: !CLOUD })
}

/** Where to go after signing in: the page that sent the user here, if it is ours. */
function returnPath(state: unknown) {
  const from = (state as { from?: unknown } | null)?.from
  return typeof from === 'string' && from.startsWith('/') && !from.startsWith('//') ? from : '/overview'
}

const passwordBytes = (value: string) => new TextEncoder().encode(value).length

function ResetPassword({ initialEmail, onDone }: { initialEmail: string; onDone: (email: string) => void }) {
  const [email, setEmail] = useState(initialEmail)
  const [requestId, setRequestId] = useState('')
  const [sentTo, setSentTo] = useState('')
  const [code, setCode] = useState('')
  const [password, setPassword] = useState('')
  const [confirm, setConfirm] = useState('')
  const [problem, setProblem] = useState('')
  const target = email.trim().toLowerCase()
  const request = useMutation({
    mutationFn: () => api<{ request_id: string }>('/auth/password-reset', jsonBody({ email: target })),
    onSuccess: result => { setRequestId(result.request_id); setSentTo(target); setCode('') },
  })
  const reset = useMutation({
    mutationFn: () => api('/auth/password-reset/confirm', jsonBody({ request_id: requestId, email: sentTo, code, new_password: password })),
    onSuccess: () => onDone(sentTo),
  })
  const busy = request.isPending || reset.isPending
  const send = (event: FormEvent) => {
    event.preventDefault()
    if (busy) return
    const message = /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(target) ? '' : '请输入有效邮箱'
    setProblem(message)
    if (!message) request.mutate()
  }
  const submit = (event: FormEvent) => {
    event.preventDefault()
    if (busy) return
    const length = passwordBytes(password)
    const message = code.length !== 6 ? '请输入 6 位验证码'
      : length < 10 || length > 256 ? '新密码长度需为 10–256 个字符'
      : password !== confirm ? '两次输入的新密码不一致'
      : ''
    setProblem(message)
    if (!message) reset.mutate()
  }
  return <div className="mt-7 grid gap-4">
    <form className="grid gap-4" onSubmit={send} noValidate>
      <Field label="登录邮箱">{id => <Input id={id} type="email" autoComplete="email" disabled={busy} value={email} onChange={event => { setEmail(event.target.value); setProblem(''); request.reset() }} placeholder="you@company.com" />}</Field>
      <Button type="submit" variant={sentTo ? 'outline' : 'default'} disabled={busy}>{request.isPending ? '发送中…' : sentTo ? '重新发送验证码' : '发送验证码'}</Button>
    </form>
    {sentTo && <form className="grid gap-4" onSubmit={submit} noValidate>
      <p role="status" className="text-sm text-muted-foreground">如果 {sentTo} 已注册，验证码会在几分钟内送达，10 分钟内有效。</p>
      <Field label="邮箱验证码">{id => <Input id={id} inputMode="numeric" autoComplete="one-time-code" maxLength={6} disabled={busy} value={code} onChange={event => { setCode(event.target.value.replace(/\D/g, '')); setProblem(''); reset.reset() }} placeholder="6 位验证码" />}</Field>
      <Field label="新密码" hint="10–256 个字符。重置后所有设备上的登录都会退出。">{id => <Input id={id} type="password" autoComplete="new-password" maxLength={256} disabled={busy} value={password} onChange={event => { setPassword(event.target.value); setProblem(''); reset.reset() }} />}</Field>
      <Field label="确认新密码">{id => <Input id={id} type="password" autoComplete="new-password" maxLength={256} disabled={busy} value={confirm} onChange={event => { setConfirm(event.target.value); setProblem(''); reset.reset() }} />}</Field>
      <Button className="h-11" type="submit" disabled={busy}>{reset.isPending ? '重置中…' : '重置密码'}</Button>
    </form>}
    {problem && <p role="alert" className="text-xs text-destructive">{problem}</p>}
    <ErrorBox error={request.error ?? reset.error} />
  </div>
}
