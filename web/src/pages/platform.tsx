import type { KeyboardEvent, ReactNode } from 'react'
import { useState } from 'react'
import { Navigate, useSearchParams } from 'react-router-dom'
import { ModelCatalogPanel } from '../components/model-catalog'
import { EmailSettingsPanel } from '../components/email-settings'
import { UnknownCostPanel } from './unknown-cost'
import { ErrorBox, Field, PageHeader, Panel } from '../components/common'
import { useUser, useAccountLimits, useRoutingSettings, queryClient } from '../lib/queries'
import { api, type AccountLimits, type RoutingSettings } from '../lib/api'
import { Button } from '../components/ui/button'
import { Checkbox } from '../components/ui/checkbox'
import { Input } from '../components/ui/input'

export function SuperAdminGuard({ children }: { children: ReactNode }) {
  const user = useUser()
  return user.is_super_admin ? children : <ErrorBox error={new Error('仅超级管理员可以访问平台管理')} />
}

export function PlatformEmailPage() {
  return <SuperAdminGuard>
    <PageHeader title="邮箱设置" description="配置发送邮箱变更验证码所需的 SMTP 服务。" />
    <EmailSettingsPanel />
  </SuperAdminGuard>
}

type ModelsTab = 'models' | 'pending' | 'unknown-cost'
const modelsTabs: { value: ModelsTab; label: string }[] = [{ value: 'models', label: '具体模型与价格' }, { value: 'pending', label: '待定价' }, { value: 'unknown-cost', label: '费用缺失' }]

export function PlatformModelsPage() {
  const [params, setParams] = useSearchParams()
  const tab = modelsTabs.find(t => t.value === params.get('tab'))?.value ?? 'models'
  const select = (next: ModelsTab) => setParams(next === 'models' ? {} : { tab: next }, { replace: true })
  // Roving tabindex with arrow / Home / End keys, per the WAI-ARIA tabs pattern.
  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    const index = modelsTabs.findIndex(t => t.value === tab)
    const next = event.key === 'ArrowRight' ? (index + 1) % modelsTabs.length : event.key === 'ArrowLeft' ? (index - 1 + modelsTabs.length) % modelsTabs.length : event.key === 'Home' ? 0 : event.key === 'End' ? modelsTabs.length - 1 : -1
    if (next < 0) return
    event.preventDefault()
    select(modelsTabs[next].value)
    document.getElementById(`models-tab-${modelsTabs[next].value}`)?.focus()
  }
  return <SuperAdminGuard>
    <PageHeader title="模型管理" description="管理模型、价格和前缀模板，查看费用缺失的请求。" />
    <div role="tablist" aria-label="模型管理" className="grid grid-cols-3 rounded-xl border border-border bg-card p-1">
      {modelsTabs.map(t => <button key={t.value} id={`models-tab-${t.value}`} type="button" role="tab" aria-selected={tab === t.value} aria-controls={`models-panel-${t.value}`} tabIndex={tab === t.value ? 0 : -1} onKeyDown={onKeyDown} onClick={() => select(t.value)} className={`rounded-lg px-4 py-3 text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring ${tab === t.value ? 'bg-primary/12 text-primary' : 'text-muted-foreground hover:bg-muted/40 hover:text-foreground'}`}>{t.label}</button>)}
    </div>
    <div role="tabpanel" id={`models-panel-${tab}`} aria-labelledby={`models-tab-${tab}`} className="mt-5">
      {tab === 'unknown-cost' ? <UnknownCostPanel onPrice={() => select('pending')} /> : <ModelCatalogPanel view={tab} onView={select} />}
    </div>
  </SuperAdminGuard>
}

export function PlatformUnknownCostPage() {
  return <Navigate to="/platform/model-catalog?tab=unknown-cost" replace />
}

function LimitsForm({ current }: { current: AccountLimits }) {
  const [values, setValues] = useState({ ...current })
  const [baseline, setBaseline] = useState({ ...current })
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<Error | null>(null)
  const [saved, setSaved] = useState(false)

  const user = useUser()
  const dirty = values.channel_limit !== baseline.channel_limit || values.virtual_key_limit !== baseline.virtual_key_limit

  function set(key: keyof AccountLimits, raw: string) {
    setSaved(false)
    const n = parseInt(raw, 10)
    if (!isNaN(n) && n >= 1) setValues(prev => ({ ...prev, [key]: n }))
    else if (raw === '' || raw === '0') setValues(prev => ({ ...prev, [key]: raw as unknown as number }))
  }

  async function submit(event: React.FormEvent) {
    event.preventDefault()
    setSaving(true)
    setError(null)
    setSaved(false)
    try {
      await api<AccountLimits>('/platform/account-limits', {
        method: 'PATCH',
        body: JSON.stringify(values),
      })
      await queryClient.invalidateQueries({ queryKey: ['account-limits', user.account_id] })
      await queryClient.invalidateQueries({ queryKey: ['account', user.account_id] })
      setBaseline({ ...values })
      setSaved(true)
    } catch (e) { setError(e as Error) } finally { setSaving(false) }
  }

  return <Panel className="mt-6 max-w-[840px] p-6">
    <form className="grid max-w-xl gap-4" onSubmit={submit}>
      <Field label="渠道数上限" hint="每个帐号可创建的上游渠道数">
        {id => <Input id={id} type="number" min={1} value={values.channel_limit} onChange={e => set('channel_limit', e.target.value)} />}
      </Field>
      <Field label="API Key 上限" hint="每个帐号可同时有效的虚拟 API Key 数">
        {id => <Input id={id} type="number" min={1} value={values.virtual_key_limit} onChange={e => set('virtual_key_limit', e.target.value)} />}
      </Field>
      <ErrorBox error={error} />
      <div className="flex flex-wrap items-center gap-3">
        <Button type="submit" disabled={saving || !dirty}>{saving ? '保存中…' : '保存设置'}</Button>
        <span role="status" aria-live="polite" className="text-sm text-success">{saved ? '已保存' : ''}</span>
      </div>
    </form>
  </Panel>
}

export function PlatformLimitsPage() {
  const limits = useAccountLimits()
  return <SuperAdminGuard>
    <PageHeader title="帐号限制" description="统一设置所有帐号的资源上限。修改后立即生效，不影响已有数据。" />
    <ErrorBox error={limits.error} />
    {limits.isPending ? <div className="h-48 animate-pulse rounded-xl bg-muted" /> : limits.data ? <LimitsForm current={limits.data} /> : null}
  </SuperAdminGuard>
}

function RoutingForm({ current }: { current: RoutingSettings }) {
  const [enabled, setEnabled] = useState(current.cooldown_enabled)
  const [maxSeconds, setMaxSeconds] = useState(String(current.max_cooldown_seconds))
  const [honorRetryAfter, setHonorRetryAfter] = useState(current.honor_retry_after)
  const [baseline, setBaseline] = useState(current)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<Error | null>(null)
  const [saved, setSaved] = useState(false)

  const user = useUser()
  const seconds = Number(maxSeconds)
  const valid = Number.isInteger(seconds) && seconds >= 30 && seconds <= 3600
  const dirty = enabled !== baseline.cooldown_enabled || seconds !== baseline.max_cooldown_seconds || honorRetryAfter !== baseline.honor_retry_after

  async function submit(event: React.FormEvent) {
    event.preventDefault()
    setSaving(true)
    setError(null)
    setSaved(false)
    try {
      const updated = await api<RoutingSettings>('/platform/routing-settings', {
        method: 'PATCH',
        body: JSON.stringify({ cooldown_enabled: enabled, max_cooldown_seconds: seconds, honor_retry_after: honorRetryAfter }),
      })
      await queryClient.invalidateQueries({ queryKey: ['routing-settings', user.account_id] })
      setBaseline(updated)
      setSaved(true)
    } catch (e) { setError(e as Error) } finally { setSaving(false) }
  }

  return <Panel className="mt-6 max-w-[840px] p-6">
    <form className="grid max-w-xl gap-5" onSubmit={submit}>
      <label className="flex items-start gap-3 text-sm">
        <Checkbox className="mt-0.5" checked={enabled} onCheckedChange={value => { setEnabled(value === true); setSaved(false) }} />
        <span className="grid gap-1"><span className="font-medium">限流冷却</span><span className="text-xs leading-5 text-muted-foreground">渠道返回 429 限流后，暂时排到其他可用渠道之后。首次冷却 30 秒，连续限流时翻倍，成功响应后重新计算。所有渠道都在冷却时仍按优先级尝试。</span></span>
      </label>
      <Field label="最长冷却时长（秒）" hint="连续限流时冷却时长翻倍的上限，30 到 3600 秒。">
        {id => <Input id={id} type="number" min={30} max={3600} disabled={!enabled} value={maxSeconds} aria-invalid={!valid} onChange={e => { setMaxSeconds(e.target.value); setSaved(false) }} />}
      </Field>
      <label className="flex items-start gap-3 text-sm">
        <Checkbox className="mt-0.5" disabled={!enabled} checked={honorRetryAfter} onCheckedChange={value => { setHonorRetryAfter(value === true); setSaved(false) }} />
        <span className="grid gap-1"><span className="font-medium">遵循 Retry-After</span><span className="text-xs leading-5 text-muted-foreground">上游返回 Retry-After 时按其时长冷却（至少 5 秒，不超过最长冷却时长），代替翻倍计时。</span></span>
      </label>
      <ErrorBox error={error} />
      <div className="flex flex-wrap items-center gap-3">
        <Button type="submit" disabled={saving || !dirty || !valid}>{saving ? '保存中…' : '保存设置'}</Button>
        <span role="status" aria-live="polite" className="text-sm text-success">{saved ? '已保存' : ''}</span>
      </div>
    </form>
  </Panel>
}

export function PlatformRoutingPage() {
  const settings = useRoutingSettings()
  return <SuperAdminGuard>
    <PageHeader title="路由设置" description="调整渠道遇到上游限流时的路由方式。保存后立即生效。" />
    <ErrorBox error={settings.error} />
    {settings.isPending ? <div className="h-48 animate-pulse rounded-xl bg-muted" /> : settings.data ? <RoutingForm current={settings.data} /> : null}
  </SuperAdminGuard>
}

export function PlatformPricingPage() {
  return <Navigate to="/platform/model-catalog" replace />
}

export function PlatformPage() {
  return <Navigate to="/platform/email-settings" replace />
}
