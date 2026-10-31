import { useState, type FormEvent } from 'react'
import { useMutation } from '@tanstack/react-query'
import { Navigate, useNavigate } from 'react-router-dom'
import { ArrowRight } from 'lucide-react'
import { api, jsonBody } from '../lib/api'
import { queryClient } from '../lib/queries'
import { Button } from '../components/ui/button'
import { Input } from '../components/ui/input'
import { ErrorBox, Field, Loading, Panel } from '../components/common'
import { BrandMark, BrandName } from '../components/brand'
import { useSetupStatus } from './auth'

const passwordBytes = (value: string) => new TextEncoder().encode(value).length

/** First run of a self-hosted instance: claims it with the token from the server log. */
export function SetupPage() {
  const status = useSetupStatus()
  const navigate = useNavigate()
  const [token, setToken] = useState('')
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [confirm, setConfirm] = useState('')
  const [problem, setProblem] = useState('')
  const setup = useMutation({
    mutationFn: () => api('/setup', jsonBody({ token: token.trim(), email: email.trim(), password })),
    onSuccess: async () => { await queryClient.cancelQueries(); queryClient.clear(); navigate('/overview', { replace: true }) },
  })
  const edit = (set: (value: string) => void) => (value: string) => { set(value); setProblem(''); setup.reset() }
  const submit = (event: FormEvent) => {
    event.preventDefault()
    if (setup.isPending) return
    const length = passwordBytes(password)
    const message = !token.trim() ? '请输入初始化令牌'
      : !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email.trim()) ? '请输入有效邮箱'
      : length < 10 || length > 256 ? '密码长度需为 10–256 个字符'
      : password !== confirm ? '两次输入的密码不一致'
      : ''
    setProblem(message)
    if (!message) setup.mutate()
  }
  if (status.isPending) return <div className="mx-auto max-w-4xl p-8"><Loading /></div>
  if (status.data && !status.data.needed && !setup.isPending && !setup.isSuccess) return <Navigate to="/login" replace />

  return <main className="auth-page grid min-h-dvh place-items-center p-4"><div className="w-full max-w-md">
    <div className="brand auth-brand mb-7 flex justify-center"><BrandMark /><BrandName /></div>
    <Panel className="p-6 sm:p-8">
      <span className="text-xs font-medium tracking-wider text-primary">FIRST RUN</span>
      <h1 className="mt-4 text-[28px] font-semibold tracking-tight">初始化实例</h1>
      <p className="mt-2 text-sm leading-6 text-muted-foreground">创建管理员帐号。初始化令牌打印在服务首次启动的日志中。</p>
      <ErrorBox error={status.error} retry={() => status.refetch()} />
      <form className="mt-7 grid gap-4" onSubmit={submit} noValidate>
        <Field label="初始化令牌">{id => <Input id={id} autoComplete="off" spellCheck={false} disabled={setup.isPending} value={token} onChange={event => edit(setToken)(event.target.value)} placeholder="setup_…" />}</Field>
        <Field label="管理员邮箱">{id => <Input id={id} type="email" autoComplete="email" disabled={setup.isPending} value={email} onChange={event => edit(setEmail)(event.target.value)} placeholder="you@company.com" />}</Field>
        <Field label="密码" hint="10–256 个字符。">{id => <Input id={id} type="password" autoComplete="new-password" maxLength={256} disabled={setup.isPending} value={password} onChange={event => edit(setPassword)(event.target.value)} />}</Field>
        <Field label="确认密码">{id => <Input id={id} type="password" autoComplete="new-password" maxLength={256} disabled={setup.isPending} value={confirm} onChange={event => edit(setConfirm)(event.target.value)} />}</Field>
        {problem && <p role="alert" className="text-xs text-destructive">{problem}</p>}
        <ErrorBox error={setup.error} />
        <Button className="mt-2 h-11" disabled={setup.isPending} type="submit">{setup.isPending ? '创建中…' : '创建管理员'}<ArrowRight aria-hidden="true" className="size-4" /></Button>
      </form>
    </Panel>
  </div></main>
}
