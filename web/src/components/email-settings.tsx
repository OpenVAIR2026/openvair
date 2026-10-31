import { useEffect, useState } from 'react'
import { useMutation, useQuery } from '@tanstack/react-query'
import { CheckCircle2, Mail, Send } from 'lucide-react'
import { api, jsonBody, type EmailSettings } from '../lib/api'
import { queryClient } from '../lib/queries'
import { Button } from './ui/button'
import { Input } from './ui/input'
import { Badge, Choice, ErrorBox, Field, Loading, Panel } from './common'

type FormState = {
  host: string
  port: string
  security: 'starttls' | 'tls'
  username: string
  password: string
  from: string
}

const emptyForm: FormState = { host: '', port: '587', security: 'starttls', username: '', password: '', from: '' }

function formFromSettings(settings: EmailSettings): FormState {
  return {
    host: settings.host,
    port: String(settings.port),
    security: settings.security,
    username: settings.username ?? '',
    password: '',
    from: settings.from,
  }
}

export function EmailSettingsPanel() {
  const settings = useQuery({ queryKey: ['platform-email-settings'], queryFn: ({ signal }) => api<EmailSettings>('/platform/email-settings', { signal }) })
  const [form, setForm] = useState<FormState>(emptyForm)
  const [saved, setSaved] = useState(false)
  const [dirty, setDirty] = useState(false)
  const [testSentTo, setTestSentTo] = useState<string | null>(null)
  useEffect(() => { if (settings.data) { setForm(formFromSettings(settings.data)); setDirty(false) } }, [settings.data])
  const save = useMutation({
    mutationFn: () => api<EmailSettings>('/platform/email-settings', { ...jsonBody({ ...form, port: Number(form.port) }, 'PATCH') }),
    onSuccess: async () => { setForm(current => ({ ...current, password: '' })); setDirty(false); setSaved(true); await queryClient.invalidateQueries({ queryKey: ['platform-email-settings'] }) },
  })
  const test = useMutation({
    mutationFn: () => api<{ sent_to: string }>('/platform/email-settings/test', { method: 'POST' }),
    onSuccess: result => setTestSentTo(result.sent_to),
  })
  const update = <K extends keyof FormState>(key: K, value: FormState[K]) => { setDirty(true); setSaved(false); setTestSentTo(null); test.reset(); setForm(current => ({ ...current, [key]: value })) }

  if (settings.isPending) return <Panel className="mt-6 max-w-[840px] p-6"><Loading /></Panel>
  if (settings.error) return <div className="mt-6 max-w-[840px]"><ErrorBox error={settings.error} retry={() => settings.refetch()} /></div>
  return <Panel className="mt-6 max-w-[840px] p-6">
    <div className="flex items-start justify-between gap-4">
      <div>
        <div className="flex items-center gap-2"><Mail aria-hidden="true" className="size-4 text-primary" /><h2 className="font-medium">SMTP 服务</h2></div>
        <p className="mt-1 text-sm leading-6 text-muted-foreground">用于发送登录邮箱变更验证码。SMTP 密码只加密保存在服务器中，不会回显。</p>
      </div>
      <Badge tone={settings.data?.configured ? 'success' : 'warning'}>{settings.data?.configured ? <><CheckCircle2 className="size-3.5" />已配置</> : '未配置'}</Badge>
    </div>
    <form className="mt-5 grid max-w-xl gap-4" onSubmit={event => { event.preventDefault(); save.mutate() }}>
      <Field label="SMTP 服务器" hint="例如 smtp.example.com">{id => <Input id={id} name="smtp-host" value={form.host} onChange={event => update('host', event.target.value)} autoComplete="off" placeholder="smtp.example.com" spellCheck={false} />}</Field>
      <Field label="端口">{id => <Input id={id} name="smtp-port" type="number" min={1} max={65535} value={form.port} onChange={event => update('port', event.target.value)} inputMode="numeric" />}</Field>
      <Field label="加密方式">{id => <Choice id={id} name="smtp-security" value={form.security} onChange={value => update('security', value as FormState['security'])} options={[{ value: 'starttls', label: 'STARTTLS（推荐）' }, { value: 'tls', label: 'TLS' }]} />}</Field>
      <Field label="SMTP 用户名" hint="无需认证时留空">{id => <Input id={id} name="smtp-username" value={form.username} onChange={event => update('username', event.target.value)} autoComplete="username" placeholder="留空表示无需认证" spellCheck={false} />}</Field>
      <Field label="SMTP 密码" hint={settings.data?.password_configured ? '留空表示保留当前密码' : '首次配置认证账号时必填'}>{id => <Input id={id} name="smtp-password" type="password" value={form.password} onChange={event => update('password', event.target.value)} autoComplete="new-password" placeholder={settings.data?.password_configured ? '留空表示保留当前密码' : '请输入 SMTP 密码'} />}</Field>
      <Field label="发件地址" hint="验证码邮件显示的发件人地址">{id => <Input id={id} name="smtp-from" type="email" value={form.from} onChange={event => update('from', event.target.value)} autoComplete="email" placeholder="noreply@example.com" spellCheck={false} />}</Field>
      <ErrorBox error={save.error ?? test.error} />
      <div className="flex flex-wrap items-center gap-3"><Button type="submit" disabled={save.isPending || test.isPending}>{save.isPending ? '保存中…' : '保存邮箱设置'}</Button><Button type="button" variant="outline" disabled={save.isPending || test.isPending || dirty || !settings.data?.configured} onClick={() => test.mutate()}><Send aria-hidden="true" className="size-4" />{test.isPending ? '发送中…' : '发送测试邮件'}</Button>{saved && <span role="status" aria-live="polite" className="text-sm text-success">已保存</span>}{testSentTo && <span role="status" aria-live="polite" className="text-sm text-success">测试邮件已发送到 {testSentTo}</span>}</div>
      <p className="text-xs leading-5 text-muted-foreground">测试邮件会发送到当前超级管理员的登录邮箱；修改配置后请先保存。</p>
    </form>
  </Panel>
}
