import { useEffect, useState, type FormEvent, type ReactNode } from 'react'
import { useMutation } from '@tanstack/react-query'
import { api, jsonBody, type AlertEmails } from '../lib/api'
import { queryClient, useAccount, useUser } from '../lib/queries'
import { ErrorBox, Loading, PageHeader, Panel } from '../components/common'
import { Button } from '../components/ui/button'
import { Input } from '../components/ui/input'
import { Checkbox } from '../components/ui/checkbox'
import { SessionsSetting } from '../components/sessions-setting'
import { ClientTokensSetting } from '../components/client-tokens-setting'
import { CLOUD } from '../lib/edition'

export function SettingsPage() {
  const user = useUser()
  const account = useAccount()
  const refreshAccount = () => Promise.all([
    queryClient.invalidateQueries({ queryKey: ['me'] }),
    queryClient.invalidateQueries({ queryKey: ['account', user.account_id] }),
  ])

  if (account.isPending) return <Loading />
  if (!account.data) return <ErrorBox error={account.error} />

  return <div className="settings-page">
    <PageHeader title="帐号设置" description="管理帐号名称、登录方式、API Key 前缀和告警邮件。" />

    <Panel className="settings-panel">
      <section className="settings-section">
        <SectionHeading title="帐号信息" description="修改邮箱或密码后，其他设备上的登录会退出。" />
        <div className="settings-items">
          <TextSetting
            id="account-name" label="帐号名称" value={account.data.name} maxLength={120} placeholder="Acme AI"
            validate={value => !value ? '请输入帐号名称' : new TextEncoder().encode(value).length > 120 ? '帐号名称不能超过 120 字节' : ''}
            save={async value => { await api('/me', jsonBody({ account_name: value }, 'PATCH')); await refreshAccount() }}
          />
          <EmailSetting email={user.email} />
          <PasswordSetting />
          <SessionsSetting />
          {CLOUD && <ClientTokensSetting />}
        </div>
      </section>
    </Panel>

    <Panel className="settings-panel">
      <section className="settings-section">
        <SectionHeading title="连接信息" description="客户端通过 API Key 连接网关。" />
        <div className="settings-items">
          <TextSetting
            id="api-key-prefix" label="API Key 前缀" value={account.data.api_key_prefix} maxLength={32} placeholder="vair"
            hint="新建 API Key 时使用，历史 Key 不受影响。1–32 个字母、数字、连字符或下划线，以字母或数字开头。"
            validate={value => /^[A-Za-z0-9][A-Za-z0-9_-]{0,31}$/.test(value) ? '' : '前缀需为 1–32 个字母、数字、连字符或下划线，并以字母或数字开头'}
            save={async value => { await api('/account', jsonBody({ api_key_prefix: value }, 'PATCH')); await refreshAccount() }}
          />
        </div>
      </section>
    </Panel>

    <Panel className="settings-panel">
      <section className="settings-section">
        <SectionHeading title="告警邮件" description={`监控发现问题时发送到 ${user.email}。每类告警只在新出现时发送一次，恢复后重新计算。需要平台已配置发信邮箱。`} />
        <AlertEmailSetting value={{ ...alertDefaults, ...account.data.alert_emails }} save={async value => { await api('/account', jsonBody({ alert_emails: value }, 'PATCH')); await refreshAccount() }} />
      </section>
    </Panel>
  </div>
}

// Accounts saved before a kind existed have no entry for it; only availability is on by default.
const alertDefaults: AlertEmails = { authenticity: false, service: false, metering: false, availability: true }

const alertKinds: { key: keyof AlertEmails; label: string; hint: string }[] = [
  ...(CLOUD ? [{ key: 'authenticity' as const, label: '真伪检测', hint: '定时抽样判为疑似不符或确认不符时' }] : []),
  { key: 'service', label: '服务体检', hint: '服务体检确认异常时' },
  { key: 'metering', label: '计量异常', hint: '真实流量中发现用量虚报或隐藏系统提示词时' },
  { key: 'availability', label: '渠道可用性', hint: '渠道持续不可用超过 30 分钟或密钥被上游拒绝时，以及之后恢复时；同一渠道每天最多告警一次' },
]

function AlertEmailSetting({ value, save }: { value: AlertEmails; save: (value: AlertEmails) => Promise<void> }) {
  const mutation = useMutation({ mutationFn: save })
  return <div className="settings-alert-list">
    {alertKinds.map(kind => <label key={kind.key} className="settings-alert-option">
      <Checkbox checked={value[kind.key]} disabled={mutation.isPending} onCheckedChange={checked => mutation.mutate({ ...value, [kind.key]: checked === true })} />
      <span className="settings-alert-text">
        <span className="settings-item-label">{kind.label}</span>
        <span className="settings-field-hint">{kind.hint}</span>
      </span>
    </label>)}
    <ErrorBox error={mutation.error} />
  </div>
}

function SectionHeading({ title, description }: { title: string; description: string }) {
  return <div className="settings-section-heading">
    <h2 className="settings-compact-title">{title}</h2>
    <p>{description}</p>
  </div>
}

/** One settings row: a label with its current value, or an editor while changing it. */
function SettingItem({ label, labelFor, editing, value, notice, children, onEdit, onCancel, busy }: {
  label: string
  labelFor?: string
  editing: boolean
  value: ReactNode
  notice?: string
  children?: ReactNode
  onEdit?: () => void
  onCancel?: () => void
  busy?: boolean
}) {
  return <div className="settings-item">
    <div className="settings-item-content">
      {editing && labelFor ? <label className="settings-item-label" htmlFor={labelFor}>{label}</label> : <span className="settings-item-label">{label}</span>}
      {editing ? children : value}
      {!editing && notice && <p className="settings-field-hint" role="status">{notice}</p>}
    </div>
    {(onEdit || onCancel) && <div className="settings-item-actions">
      {editing
        ? <Button type="button" variant="ghost" size="sm" disabled={busy} onClick={onCancel}>取消</Button>
        : <Button type="button" variant="outline" size="sm" aria-label={`修改${label}`} onClick={onEdit}>修改</Button>}
    </div>}
  </div>
}

// Warn before leaving the page while an editor holds unsaved input.
function useUnsavedWarning(active: boolean) {
  useEffect(() => {
    if (!active) return
    const warn = (event: BeforeUnloadEvent) => { event.preventDefault(); event.returnValue = '' }
    window.addEventListener('beforeunload', warn)
    return () => window.removeEventListener('beforeunload', warn)
  }, [active])
}

function TextSetting({ id, label, value, hint, maxLength, placeholder, validate, save }: {
  id: string
  label: string
  value: string
  hint?: string
  maxLength: number
  placeholder: string
  validate: (value: string) => string
  save: (value: string) => Promise<void>
}) {
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState('')
  const [problem, setProblem] = useState('')
  const [notice, setNotice] = useState('')
  const mutation = useMutation({ mutationFn: save, onSuccess: () => { setEditing(false); setNotice('已保存') } })
  useUnsavedWarning(editing && draft.trim() !== value)
  const submit = (event: FormEvent) => {
    event.preventDefault()
    const next = draft.trim()
    if (next === value) { setEditing(false); return }
    const message = validate(next)
    setProblem(message)
    if (!message) mutation.mutate(next)
  }
  return <SettingItem
    label={label} labelFor={id} editing={editing} busy={mutation.isPending} notice={notice}
    value={<><p className="settings-item-value">{value || '未设置'}</p>{hint && <p className="settings-field-hint">{hint}</p>}</>}
    onEdit={() => { setDraft(value); setProblem(''); setNotice(''); mutation.reset(); setEditing(true) }}
    onCancel={() => setEditing(false)}
  >
    <form className="settings-inline-form" onSubmit={submit} noValidate>
      <div className="settings-email-row">
        <Input id={id} autoComplete="off" maxLength={maxLength} placeholder={placeholder} disabled={mutation.isPending} value={draft}
          onChange={event => { setDraft(event.target.value); setProblem(''); mutation.reset() }}
          onKeyDown={event => { if (event.key === 'Escape') setEditing(false) }} />
        <Button type="submit" disabled={mutation.isPending}>{mutation.isPending ? '保存中…' : '保存'}</Button>
      </div>
      {hint && <p className="settings-field-hint">{hint}</p>}
      {problem && <p role="alert" className="text-xs text-destructive">{problem}</p>}
      <ErrorBox error={mutation.error} />
    </form>
  </SettingItem>
}

const EMAIL_PATTERN = /^[^\s@]+@[^\s@]+\.[^\s@]+$/

function EmailSetting({ email }: { email: string }) {
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState('')
  const [password, setPassword] = useState('')
  const [code, setCode] = useState('')
  const [requestId, setRequestId] = useState('')
  const [sentTo, setSentTo] = useState('')
  const [cooldown, setCooldown] = useState(0)
  const [problem, setProblem] = useState('')
  const [notice, setNotice] = useState('')
  const target = draft.trim().toLowerCase()

  useEffect(() => {
    if (!cooldown) return
    const timer = window.setInterval(() => setCooldown(value => Math.max(0, value - 1)), 1000)
    return () => window.clearInterval(timer)
  }, [cooldown])
  useUnsavedWarning(editing && (!!password || !!code || (!!target && target !== email)))

  const request = useMutation({
    mutationFn: () => api<{ request_id: string; expires_in: number; retry_after?: number }>('/me/email-verification', jsonBody({ email: target, current_password: password })),
    onSuccess: result => { setRequestId(result.request_id); setSentTo(target); setCode(''); setCooldown(result.retry_after ?? 60) },
  })
  const confirm = useMutation({
    mutationFn: () => api('/me/email-verification/confirm', jsonBody({ request_id: requestId, email: sentTo, code })),
    onSuccess: async () => {
      const updated = sentTo
      close()
      setNotice(`登录邮箱已更新为 ${updated}，其他设备上的登录已退出。`)
      await queryClient.invalidateQueries({ queryKey: ['me'] })
    },
  })
  const busy = request.isPending || confirm.isPending
  const close = () => {
    setEditing(false); setDraft(''); setPassword(''); setCode(''); setRequestId(''); setSentTo(''); setProblem('')
    request.reset(); confirm.reset()
  }
  // The code is bound to the address it was sent to; changing the address needs a new code.
  const changeDraft = (value: string) => {
    setDraft(value); setProblem(''); request.reset(); confirm.reset()
    if (sentTo && value.trim().toLowerCase() !== sentTo) { setRequestId(''); setSentTo(''); setCode('') }
  }
  const send = (event: FormEvent) => {
    event.preventDefault()
    if (busy || cooldown > 0) return
    const message = !EMAIL_PATTERN.test(target) ? '请输入有效的邮箱地址'
      : target === email ? '请输入与当前邮箱不同的新邮箱'
      : !password ? '请输入当前密码以确认身份'
      : ''
    setProblem(message)
    if (!message) request.mutate()
  }
  const submitCode = (event: FormEvent) => {
    event.preventDefault()
    if (busy) return
    const message = code.length === 6 ? '' : '请输入 6 位验证码'
    setProblem(message)
    if (!message) confirm.mutate()
  }

  return <SettingItem
    label="登录邮箱" labelFor="email-draft" editing={editing} busy={busy} notice={notice}
    value={<p className="settings-item-value">{email}</p>}
    onEdit={() => { close(); setNotice(''); setEditing(true) }}
    onCancel={close}
  >
    <div className="settings-email-editor">
      <form className="settings-inline-form" onSubmit={send} noValidate>
        <Input id="email-draft" name="email" type="email" autoComplete="email" spellCheck={false} disabled={busy} value={draft} onChange={event => changeDraft(event.target.value)} placeholder="新的登录邮箱" />
        <label className="settings-item-label" htmlFor="email-current-password">当前密码</label>
        <Input id="email-current-password" name="current_password" type="password" autoComplete="current-password" disabled={busy} value={password} onChange={event => { setPassword(event.target.value); setProblem(''); request.reset() }} placeholder="输入当前密码以确认身份" />
        <div>
          <Button type="submit" variant={sentTo ? 'outline' : 'default'} disabled={busy || cooldown > 0}>
            {request.isPending ? '发送中…' : cooldown > 0 ? `${cooldown}s 后可重发` : sentTo ? '重新发送' : '发送验证码'}
          </Button>
        </div>
      </form>
      {sentTo && <form className="settings-inline-form" onSubmit={submitCode} noValidate>
        <p className="settings-field-hint" role="status">验证码已发送至 {sentTo}，10 分钟内有效。</p>
        <label className="settings-item-label" htmlFor="email-code">邮箱验证码</label>
        <div className="settings-email-row">
          <Input id="email-code" name="email_verification_code" inputMode="numeric" autoComplete="one-time-code" maxLength={6} disabled={busy} value={code} onChange={event => { setCode(event.target.value.replace(/\D/g, '')); setProblem(''); confirm.reset() }} placeholder="6 位验证码" />
          <Button type="submit" disabled={busy}>{confirm.isPending ? '确认中…' : '确认修改'}</Button>
        </div>
      </form>}
      {problem && <p role="alert" className="text-xs text-destructive">{problem}</p>}
      <ErrorBox error={request.error ?? confirm.error} />
    </div>
  </SettingItem>
}

// The server limits password length in UTF-8 bytes.
const passwordBytes = (value: string) => new TextEncoder().encode(value).length

function PasswordSetting() {
  const [editing, setEditing] = useState(false)
  const [current, setCurrent] = useState('')
  const [next, setNext] = useState('')
  const [confirm, setConfirm] = useState('')
  const [problem, setProblem] = useState('')
  const [notice, setNotice] = useState('')
  const change = useMutation({
    mutationFn: () => api('/me/password', jsonBody({ current_password: current, new_password: next })),
    onSuccess: () => { close(); setNotice('密码已更新，其他设备上的登录已退出。') },
  })
  useUnsavedWarning(editing && !!(current || next || confirm))
  const close = () => { setEditing(false); setCurrent(''); setNext(''); setConfirm(''); setProblem(''); change.reset() }
  const submit = (event: FormEvent) => {
    event.preventDefault()
    if (change.isPending) return
    const length = passwordBytes(next)
    const message = !current ? '请输入当前密码'
      : length < 10 || length > 256 ? '新密码长度需为 10–256 个字符'
      : next !== confirm ? '两次输入的新密码不一致'
      : next === current ? '新密码不能与当前密码相同'
      : ''
    setProblem(message)
    if (!message) change.mutate()
  }
  const field = (id: string, label: string, value: string, setter: (value: string) => void, autoComplete: string) => <>
    <label className="settings-item-label" htmlFor={id}>{label}</label>
    <Input id={id} type="password" autoComplete={autoComplete} maxLength={256} disabled={change.isPending} value={value} onChange={event => { setter(event.target.value); setProblem(''); change.reset() }} />
  </>

  return <SettingItem
    label="登录密码" editing={editing} busy={change.isPending} notice={notice}
    value={<p className="settings-item-value">已设置</p>}
    onEdit={() => { setNotice(''); setEditing(true) }}
    onCancel={close}
  >
    <form className="settings-password-editor" onSubmit={submit} noValidate>
      {field('password-current', '当前密码', current, setCurrent, 'current-password')}
      {field('password-new', '新密码', next, setNext, 'new-password')}
      {field('password-confirm', '确认新密码', confirm, setConfirm, 'new-password')}
      <p className="settings-field-hint">10–256 个字符。更新后其他设备上的登录会退出，当前设备保持登录。</p>
      {problem && <p role="alert" className="text-xs text-destructive">{problem}</p>}
      <ErrorBox error={change.error} />
      <div><Button type="submit" disabled={change.isPending}>{change.isPending ? '更新中…' : '更新密码'}</Button></div>
    </form>
  </SettingItem>
}
