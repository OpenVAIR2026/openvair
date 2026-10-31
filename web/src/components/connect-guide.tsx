import { Fragment, useEffect, useRef, useState } from 'react'
import { Check, CircleAlert, CircleCheck, Code2, Copy, Eye, EyeOff, LoaderCircle, PlugZap } from 'lucide-react'
import { Link } from 'react-router-dom'
import { api, type VirtualKey } from '../lib/api'
import { copyText } from '../lib/clipboard'
import { clientSnippets, exampleModel, KEY_PLACEHOLDER, type ClientId } from '../lib/client-snippets'
import { markConnectionVerified, testConnection, type ConnectionResult } from '../lib/onboarding'
import { useOrigin, useUser } from '../lib/queries'
import { Choice, ErrorBox, Loading } from './common'
import { Button } from './ui/button'
import { DesktopShortcut } from './desktop-promo'

const modelPrefix: Partial<Record<ClientId, string>> = { 'claude-code': 'claude-', 'gemini-cli': 'gemini-', codex: 'gpt-', generic: 'gpt-', curl: 'gpt-' }

// Marks where the key goes in a snippet, so it can be shown as a labelled placeholder instead of a partial key.
const keyToken = '\u0000vair-key\u0000'

export const usableKeys = (keys: VirtualKey[] | undefined) => (keys ?? []).filter(key => key.enabled && !key.revoked_at)

/**
 * Shows how to point a client at the gateway with one of the account's keys.
 * The page only ever renders the key prefix; the full key is fetched when copying or testing.
 */
export function ConnectGuide({ keys, initialKeyId, secret, showSecret = false }: { keys: VirtualKey[]; initialKeyId?: string; secret?: { id: string; value: string }; showSecret?: boolean }) {
  const user = useUser()
  const usable = usableKeys(keys)
  // Follows the initial key until the user picks one, so a key that is still loading is selected once it arrives.
  const [chosen, setKeyId] = useState<string>()
  const keyId = chosen ?? usable.find(key => key.id === initialKeyId)?.id ?? usable[0]?.id ?? ''
  const [clientId, setClientId] = useState<ClientId>('claude-code')
  const [copy, setCopy] = useState<'idle' | 'copied' | 'placeholder' | 'error' | 'reveal-error'>('idle')
  const [shownId, setShownId] = useState<string | null>(null)
  const [revealing, setRevealing] = useState(false)
  const [testing, setTesting] = useState(false)
  const [result, setResult] = useState<ConnectionResult | null>(null)
  const revealed = useRef(new Map<string, string>())
  const abort = useRef<AbortController | null>(null)
  useEffect(() => () => abort.current?.abort(), [])
  const key = usable.find(item => item.id === keyId)
  const models = key?.known_models ?? []
  const origin = useOrigin()
  const model = exampleModel(models, modelPrefix[clientId])
  // A just-created key is already on screen in full; other keys stay hidden until the user asks to show them.
  const visibleKey = key && (secret?.id === key.id ? secret.value : shownId === key.id ? revealed.current.get(key.id) : undefined)
  const fillable = !!key && (secret?.id === key.id || key.can_copy)
  const snippets = clientSnippets({ origin, key: visibleKey ?? (fillable ? keyToken : undefined), model }, { curl: true })
  const snippet = snippets.find(item => item.id === clientId) ?? snippets[0]
  const missing = snippet.expects && models.length > 0 && !models.some(name => name.startsWith(snippet.expects!.prefix))

  const saveStep = showSecret && secret && secret.id === (key?.id ?? initialKeyId) ? <SaveKey value={secret.value} /> : null
  // A key just created may not be in the refreshed list yet; its full value must stay on screen meanwhile.
  if (!key && saveStep) return <div className="grid gap-4">{saveStep}<Loading /></div>
  if (!key) return <div className="rounded-lg border border-dashed border-border p-4 text-sm text-muted-foreground">还没有可用的 API Key。<Link to="/keys?create=1" className="text-primary hover:underline">创建 API Key</Link> 后即可查看接入方式。</div>

  async function fullKey(target: VirtualKey) {
    if (secret?.id === target.id) return secret.value
    const cached = revealed.current.get(target.id)
    if (cached) return cached
    if (!target.can_copy) return null
    const { key: value } = await api<{ key: string }>(`/keys/${target.id}/reveal`, { method: 'POST', cache: 'no-store' })
    revealed.current.set(target.id, value)
    return value
  }

  function select(next: ClientId) { setClientId(next); setCopy('idle') }
  function selectKey(next: string) { setKeyId(next); setShownId(null); setCopy('idle'); setResult(null) }

  async function showKey() {
    setRevealing(true)
    try {
      if (await fullKey(key!)) { setShownId(key!.id); setCopy('idle') }
    } catch {
      setCopy('reveal-error')
    } finally {
      setRevealing(false)
    }
  }

  async function copySnippet() {
    try {
      const value = await fullKey(key!)
      const code = clientSnippets({ origin, key: value ?? KEY_PLACEHOLDER, model }, { curl: true }).find(item => item.id === clientId)!.code
      await copyText(code)
      setCopy(value ? 'copied' : 'placeholder')
    } catch {
      setCopy('error')
    }
  }

  async function runTest() {
    abort.current?.abort()
    const controller = new AbortController()
    abort.current = controller
    setTesting(true); setResult(null)
    try {
      const value = await fullKey(key!)
      if (!value) { setResult({ status: 'error', message: '这把 Key 创建时只保存了哈希，无法在网页中测试。请直接在客户端发起一次调用。' }); return }
      const outcome = await testConnection(value, controller.signal)
      setResult(outcome)
      if (outcome.status === 'ok' && outcome.models > 0) markConnectionVerified(user.account_id)
    } catch (error) {
      if (!controller.signal.aborted) setResult({ status: 'error', message: error instanceof Error ? error.message : '测试失败，请重试。' })
    } finally {
      if (!controller.signal.aborted) setTesting(false)
    }
  }

  return <div className="connect-guide grid gap-4">
    {saveStep}
    {saveStep && <h3 className="text-sm font-medium">接入客户端</h3>}
    {usable.length > 1 && <div className="grid gap-2"><label htmlFor="connect-guide-key" className="text-sm font-medium">使用的 API Key</label><Choice id="connect-guide-key" value={keyId} onChange={selectKey} options={usable.map(item => ({ value: item.id, label: usable.some(other => other.id !== item.id && other.name === item.name) ? `${item.name}（${item.key_prefix}）` : item.name }))} /></div>}
    <dl className="grid gap-2 rounded-lg border border-border p-3 text-sm sm:grid-cols-[auto_1fr] sm:gap-x-4">
      <dt className="text-muted-foreground">接口地址</dt><dd className="min-w-0 break-all"><code>{origin}</code><span className="text-muted-foreground">（OpenAI 兼容客户端加 <code>/v1</code>）</span></dd>
      <dt className="text-muted-foreground">可用模型</dt><dd className="min-w-0">{models.length ? <span className="break-words" title={models.join('、')}>{models.slice(0, 4).join('、')}{models.length > 4 ? ` 等 ${models.length} 个` : ''}</span> : <span className="text-warning">暂无可路由的模型</span>}</dd>
    </dl>
    {!models.length && <p className="flex gap-2 rounded-md bg-warning/10 p-3 text-sm text-warning"><CircleAlert aria-hidden="true" className="mt-0.5 size-4 shrink-0" /><span>这把 Key 当前没有可调用的模型。请先<Link to="/channels" className="underline">添加或启用渠道</Link>，或在 Key 设置中放开模型范围。</span></p>}
    <div className="overflow-hidden rounded-lg border border-border">
      <div className="flex gap-1 overflow-x-auto border-b border-border bg-muted/30 p-1" role="tablist" aria-label="选择客户端" onKeyDown={event => {
        if (event.key !== 'ArrowRight' && event.key !== 'ArrowLeft') return
        const index = snippets.findIndex(item => item.id === clientId)
        const next = (index + (event.key === 'ArrowRight' ? 1 : -1) + snippets.length) % snippets.length
        select(snippets[next].id)
        event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="tab"]')[next]?.focus()
      }}>
        {snippets.map(item => <button key={item.id} type="button" role="tab" id={`connect-tab-${item.id}`} aria-selected={item.id === clientId} aria-controls="connect-tabpanel" tabIndex={item.id === clientId ? 0 : -1} onClick={() => select(item.id)} className="whitespace-nowrap rounded-md px-3 py-1.5 text-sm text-muted-foreground hover:text-foreground aria-selected:bg-background aria-selected:font-medium aria-selected:text-foreground aria-selected:shadow-sm">{item.name}</button>)}
      </div>
      <div id="connect-tabpanel" role="tabpanel" aria-labelledby={`connect-tab-${snippet.id}`} className="grid gap-3 p-3">
        <div className="flex items-center justify-between gap-3">
          <span className="inline-flex items-center gap-1.5 text-xs text-muted-foreground"><Code2 aria-hidden="true" className="size-3.5" />{snippet.label}</span>
          <div className="flex items-center gap-1">
            {fillable && secret?.id !== key.id && (visibleKey
              ? <Button type="button" variant="ghost" size="sm" onClick={() => setShownId(null)}><EyeOff aria-hidden="true" />隐藏 Key</Button>
              : <Button type="button" variant="ghost" size="sm" disabled={revealing} onClick={showKey}><Eye aria-hidden="true" />{revealing ? '读取中…' : '显示 Key'}</Button>)}
            <Button type="button" variant="outline" size="sm" onClick={copySnippet}>{copy === 'copied' ? <Check aria-hidden="true" /> : <Copy aria-hidden="true" />}{copy === 'copied' ? '已复制' : '复制配置'}</Button>
          </div>
        </div>
        <pre tabIndex={0} aria-label={`${snippet.name} 接入配置`} className="overflow-x-auto rounded-md bg-muted/50 p-3 text-xs leading-5"><code>{snippet.code.split(keyToken).map((part, index) => <Fragment key={index}>{index > 0 && <span className="connect-key-token" title="点击「复制配置」时自动填入完整 API Key">复制时自动填入 Key</span>}{part}</Fragment>)}</code></pre>
        <p className="text-xs leading-5 text-muted-foreground" role="status">{copy === 'copied' ? '已复制，配置中包含完整 API Key，请勿分享。' : copy === 'placeholder' ? `已复制。这把 Key 无法恢复完整值，请把配置中的「${KEY_PLACEHOLDER}」替换为保存的 Key。` : copy === 'error' ? '复制失败，请重试或手动选择上方内容。' : copy === 'reveal-error' ? '无法读取完整 API Key，请重试。' : snippet.note}</p>
        {missing && <p className="flex gap-2 rounded-md bg-warning/10 p-3 text-sm text-warning"><CircleAlert aria-hidden="true" className="mt-0.5 size-4 shrink-0" /><span>{snippet.expects!.hint}</span></p>}
      </div>
    </div>
    <div className="grid gap-3 rounded-lg border border-border p-3">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div><p className="text-sm font-medium">测试连接</p><p className="mt-0.5 text-xs text-muted-foreground">用这把 Key 查询可用模型，不调用上游，不产生费用。</p></div>
        <Button type="button" variant="outline" size="sm" disabled={testing} onClick={runTest}>{testing ? <LoaderCircle aria-hidden="true" className="animate-spin" /> : <PlugZap aria-hidden="true" />}{testing ? '测试中…' : '测试连接'}</Button>
      </div>
      {result && <ConnectionFeedback result={result} />}
    </div>
    <DesktopShortcut />
  </div>
}

function ConnectionFeedback({ result }: { result: ConnectionResult }) {
  const [tone, text] = result.status === 'ok'
    ? result.models > 0
      ? ['success', `连接正常，这把 Key 可调用 ${result.models} 个模型。在客户端发起请求后，可在使用分析中查看记录。`] as const
      : ['warning', '连接正常，但这把 Key 当前没有可调用的模型。请检查渠道是否启用，以及 Key 的渠道和模型范围。'] as const
    : result.status === 'unauthorized'
      ? ['danger', 'Key 无效、已停用或已撤销。请确认使用的是完整 Key，并在列表中检查状态。'] as const
      : ['danger', result.message] as const
  const Icon = tone === 'success' ? CircleCheck : CircleAlert
  return <p role="status" className={`flex gap-2 rounded-md p-3 text-sm ${{ success: 'bg-success/10 text-success', warning: 'bg-warning/10 text-warning', danger: 'bg-destructive/10 text-destructive' }[tone]}`}><Icon aria-hidden="true" className="mt-0.5 size-4 shrink-0" /><span>{text}</span></p>
}

function SaveKey({ value }: { value: string }) {
  const [copied, setCopied] = useState(false)
  const [error, setError] = useState<Error | null>(null)
  return <section className="grid gap-3 rounded-lg border border-primary/30 bg-primary/5 p-3" aria-label="保存完整 API Key">
    <div className="flex items-center justify-between gap-3">
      <h3 className="text-sm font-medium">保存完整 API Key</h3>
      <Button type="button" variant="outline" size="sm" onClick={async () => { try { await copyText(value); setError(null); setCopied(true) } catch { setError(new Error('无法访问剪贴板，请手动选择并复制完整 API Key。')) } }}>{copied ? <Check aria-hidden="true" /> : <Copy aria-hidden="true" />}{copied ? '已复制' : '复制 Key'}</Button>
    </div>
    <code className="block break-all rounded-md bg-background p-3 text-sm">{value}</code>
    <ErrorBox error={error} />
    <p className="text-xs leading-5 text-muted-foreground">请保存到安全的位置，不要分享给他人。关闭后仍可在列表中再次复制。</p>
  </section>
}
