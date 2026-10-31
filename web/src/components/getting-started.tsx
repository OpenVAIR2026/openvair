import { useEffect, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { ArrowRight, Check, X } from 'lucide-react'
import { Link } from 'react-router-dom'
import { api, type UsageEventPage } from '../lib/api'
import { connectionVerified, dismissOnboarding, onboardingDismissed } from '../lib/onboarding'
import { useChannels, useKeys, useUser } from '../lib/queries'
import { ConnectGuide, usableKeys } from './connect-guide'
import { FormSheet } from './common'
import { Button } from './ui/button'

/** Progress through first-time setup, derived from existing resources; nothing is stored on the server. */
export function useOnboarding() {
  const user = useUser()
  const channels = useChannels()
  const keys = useKeys()
  const [, rerender] = useState(0)
  useEffect(() => {
    const update = () => rerender(value => value + 1)
    window.addEventListener('vair:onboarding', update)
    return () => window.removeEventListener('vair:onboarding', update)
  }, [])
  const dismissed = onboardingDismissed(user.account_id)
  const usable = usableKeys(keys.data)
  const hasModels = (channels.data ?? []).some(channel => channel.enabled) || usable.some(key => key.known_models?.length)
  const hasKey = usable.length > 0
  const offset = -new Date().getTimezoneOffset()
  const events = useQuery({
    queryKey: ['usage-events', user.account_id, 'onboarding'],
    queryFn: ({ signal }) => api<UsageEventPage>(`/usage/events?range=all&utc_offset_minutes=${offset}&page=1&limit=1`, { signal }),
    enabled: !dismissed && hasKey,
  })
  const connected = connectionVerified(user.account_id) || (events.data?.total ?? 0) > 0
  const ready = channels.isSuccess && keys.isSuccess && (!hasKey || connected || !events.isPending)
  const steps = [hasModels, hasKey, connected]
  return { visible: ready && !dismissed && steps.some(done => !done), steps, keys: keys.data ?? [], dismiss: () => dismissOnboarding(user.account_id) }
}

const stepCopy = [
  { title: '添加渠道', text: '接入供应商 API 或已获授权的 Key，让网关有可调用的模型。', action: '添加渠道', to: '/channels?add=1' },
  { title: '创建 API Key', text: '客户端使用 API Key 调用网关，可按需限制渠道、模型和用量。', action: '创建 API Key', to: '/keys?create=1' },
  { title: '接入客户端', text: '复制配置到 Claude Code、Codex 等工具，并测试连接。', action: '查看接入方式' },
]

export function GettingStarted({ onboarding }: { onboarding: ReturnType<typeof useOnboarding> }) {
  const [guideOpen, setGuideOpen] = useState(false)
  const { steps, keys, dismiss } = onboarding
  const current = steps.findIndex(done => !done)
  const completed = steps.filter(Boolean).length
  // Keep the guide open after a successful test even though that completes the last step.
  if (!onboarding.visible && !guideOpen) return null
  return <section className="getting-started" aria-labelledby="getting-started-title">
    <div className="getting-started-heading">
      <div><h2 id="getting-started-title">开始使用<span>已完成 {completed} / {steps.length}</span></h2><p>完成以下步骤，从客户端发起第一次调用。</p></div>
      <Button variant="ghost" size="icon-sm" aria-label="隐藏开始使用" title="隐藏" onClick={dismiss}><X aria-hidden="true" /></Button>
    </div>
    <ol className="getting-started-steps">{stepCopy.map((step, index) => {
      const done = steps[index]
      const active = index === current
      const blocked = index === 2 && !steps[1]
      return <li key={step.title} data-state={done ? 'done' : active ? 'active' : 'pending'} aria-current={active ? 'step' : undefined}>
        <span className="getting-started-marker" aria-hidden="true">{done ? <Check /> : index + 1}</span>
        <div className="min-w-0">
          <h3>{step.title}{done && <span className="sr-only">（已完成）</span>}</h3>
          <p>{step.text}</p>
          {!done && (step.to
            ? <Button asChild size="sm" variant={active ? 'default' : 'outline'}><Link to={step.to}>{step.action}<ArrowRight aria-hidden="true" /></Link></Button>
            : <Button size="sm" variant={active ? 'default' : 'outline'} disabled={blocked} title={blocked ? '请先创建 API Key' : undefined} onClick={() => setGuideOpen(true)}>{step.action}<ArrowRight aria-hidden="true" /></Button>)}
        </div>
      </li>
    })}</ol>
    <FormSheet open={guideOpen} onOpenChange={setGuideOpen} title="接入客户端" description="复制配置到你使用的工具，然后测试连接。">
      {guideOpen && <ConnectGuide keys={keys} />}
    </FormSheet>
  </section>
}
