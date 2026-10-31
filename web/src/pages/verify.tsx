// Public model verification: /verify submits one pasted endpoint without an
// account, /verify/r/:token shows its report.
import { useEffect, useMemo, useState, type CSSProperties, type FormEvent } from 'react'
import { useMutation, useQuery } from '@tanstack/react-query'
import { Link, useNavigate, useParams } from 'react-router-dom'
import { api, ApiError, jsonBody, formatDate, type Protocol, type User } from '../lib/api'
import { copyText } from '../lib/clipboard'
import { SiteHeader } from '../components/site-header'
import './verify.css'

interface PublicModel { model: string; family: string; estimated_cost_usd: number | null }
interface PublicModels { models: PublicModel[]; target_samples: number }
interface Share { model: string; share: number; claimed: boolean }
interface SampleRow { result: string; latency_ms: number; leaning: string | null }
interface Evidence {
  claimed_family: string
  closest_model: string | null
  shares: Share[]
  vendor: { consistent: boolean | null; families: string[] }
  model: { closest: string | null; same: boolean; closest_same_family: boolean | null }
  tokens: { surplus: number | null }
  samples: SampleRow[]
}
interface Report {
  token: string
  status: 'queued' | 'running' | 'done' | 'failed'
  protocol: Protocol
  host: string | null
  show_host: boolean
  owner: boolean
  claimed_model: string
  target_samples: number
  completed_samples: number
  valid_samples: number
  queue_ahead: number | null
  verdict: string | null
  progress: { shares: Share[] } | null
  evidence: Evidence | null
  error: string | null
  cost_usd: number | null
  created_at: string
  started_at: string | null
  finished_at: string | null
}

const protocols: { value: Protocol; label: string }[] = [
  { value: 'openai', label: 'OpenAI 兼容' },
  { value: 'anthropic', label: 'Anthropic' },
  { value: 'gemini', label: 'Gemini' },
]

const vendors: Record<string, { name: string; series: string; color: string }> = {
  anthropic: { name: 'Anthropic', series: 'Claude', color: '#e8a77b' },
  openai: { name: 'OpenAI', series: 'GPT', color: '#80d7a3' },
  deepseek: { name: 'DeepSeek', series: 'DeepSeek', color: '#8fb4ff' },
  google: { name: 'Google', series: 'Gemini', color: '#b9a3f5' },
}
const vendorOf = (family: string) => vendors[family.toLowerCase()] ?? { name: family, series: family, color: '#8fa4a9' }

const manageKey = (token: string) => `vair-verify-manage:${token}`
const errorText = (error: unknown) => error instanceof ApiError ? error.message : '请求失败，请稍后重试'
const usd = (value: number | null | undefined) => value == null ? '—' : `≈ $${value < 0.01 ? value.toFixed(3) : value.toFixed(2)}`

function VerifyFooter() {
  return <footer className="vf-footer"><span>VAIR · 你自己的渠道，你自己的 Key</span><span>VAIR 不出售额度，不为任何渠道背书</span></footer>
}

function Backdrop() {
  return <><div className="vf-glow" aria-hidden="true" /><div className="vf-grid" aria-hidden="true" /></>
}

// ------------------------------------------------------------------ form page

interface Draft { protocol: Protocol; base_url: string; api_key: string; model: string; upstream_model: string }

const faqs: [string, string][] = [
  ['为什么不直接问模型“你是谁”？', '自我介绍可以被系统提示词改写。VAIR 比较回答的统计特征和 Token 计数，这些很难伪装。'],
  ['检测要花多少钱？', '每次最多 16 个请求，大多是很短的回答，证据足够时会提前结束。费用由你填的这把 Key 承担，确认页会显示按官方单价估算的金额。'],
  ['我的 Key 安全吗？', 'Key 和接口地址只加密暂存在缓存里，最长 15 分钟，检测结束、失败或超时都会立即删除。数据库和日志不保存 Key，报告只记录域名，而且默认不公开。'],
  ['结论有哪几种？', '与申报模型相符、确认厂商但型号难分、疑似不是申报的模型、确认不是申报的模型、结果矛盾、证据不足。每种结论都会附上关键证据。'],
  ['渠道会发现我在检测吗？', '有可能。检测请求和日常请求不同，渠道如果有意识别是能认出来的。所以一次相符不代表一直相符，想长期观察可以注册后给渠道开启定时检测。'],
  ['报告会公开吗？', '报告只能通过链接打开，不会被搜索引擎收录，180 天后自动删除。报告不显示渠道域名，提交检测的浏览器可以随时删除它。'],
]

export function VerifyPage() {
  const navigate = useNavigate()
  const models = useQuery({ queryKey: ['public-verify-models'], queryFn: ({ signal }) => api<PublicModels>('/public/verify/models', { signal }) })
  const [draft, setDraft] = useState<Draft>({ protocol: 'openai', base_url: '', api_key: '', model: '', upstream_model: '' })
  const [advanced, setAdvanced] = useState(false)
  const [formError, setFormError] = useState('')
  const [confirming, setConfirming] = useState(false)
  const set = (patch: Partial<Draft>) => { setDraft(d => ({ ...d, ...patch })); setFormError('') }

  const groups = useMemo(() => {
    const map = new Map<string, PublicModel[]>()
    for (const m of models.data?.models ?? []) map.set(m.family, [...(map.get(m.family) ?? []), m])
    return [...map.entries()]
  }, [models.data])
  const chosen = models.data?.models.find(m => m.model === draft.model)

  function review(event: FormEvent) {
    event.preventDefault()
    if (!/^https:\/\/[^\s/]+/i.test(draft.base_url.trim())) return setFormError('接口地址必须以 https:// 开头')
    if (!draft.api_key.trim()) return setFormError('请填写 API Key')
    if (!draft.model) return setFormError('请选择申报模型')
    setConfirming(true)
  }

  return <div className="vf-page">
    <Backdrop />
    <SiteHeader />
    <main className="vf-main">
      <section className="vf-hero">
        <div className="vf-tag">MODEL VERIFY</div>
        <h1>你用的模型，<br /><span>真的是那个模型吗？</span></h1>
        <p>填入渠道信息，几分钟拿到一份检测报告。免注册。</p>
      </section>

      <form className="vf-frame" onSubmit={review} noValidate>
        <div className="vf-card vf-form">
          <div className="vf-field vf-full">
            <span className="vf-label"><i>01</i>协议</span>
            <div className="vf-seg" role="radiogroup" aria-label="协议">
              {protocols.map(p => <button type="button" key={p.value} role="radio" aria-checked={draft.protocol === p.value} className={draft.protocol === p.value ? 'on' : ''} onClick={() => set({ protocol: p.value })}>{p.label}</button>)}
            </div>
          </div>
          <label className="vf-field vf-full">
            <span className="vf-label"><i>02</i>接口地址<small>只支持 HTTPS</small></span>
            <input className="vf-input mono" inputMode="url" autoComplete="off" spellCheck={false} placeholder="https://" value={draft.base_url} onChange={e => set({ base_url: e.target.value })} />
          </label>
          <label className="vf-field">
            <span className="vf-label"><i>03</i>API Key</span>
            <input className="vf-input mono" type="password" autoComplete="off" spellCheck={false} placeholder="sk-…" value={draft.api_key} onChange={e => set({ api_key: e.target.value })} />
          </label>
          <label className="vf-field">
            <span className="vf-label"><i>04</i>申报模型</span>
            <select className="vf-input" value={draft.model} onChange={e => set({ model: e.target.value })} disabled={!models.data}>
              <option value="">{models.isError ? '模型列表加载失败' : '选择模型'}</option>
              {groups.map(([family, list]) => <optgroup key={family} label={vendorOf(family).name}>
                {list.map(m => <option key={m.model} value={m.model}>{m.model}</option>)}
              </optgroup>)}
            </select>
          </label>
          <div className="vf-adv vf-full">
            <button type="button" onClick={() => setAdvanced(v => !v)} aria-expanded={advanced}>
              <span>高级选项 <b>· 渠道里的模型名与申报不同</b></span><span className="vf-link">{advanced ? '收起' : '展开'}</span>
            </button>
            {advanced && <label className="vf-field">
              <span className="vf-label">渠道里的模型名<small>不填则使用申报模型</small></span>
              <input className="vf-input mono" autoComplete="off" spellCheck={false} placeholder={draft.model || '例如 my-opus'} value={draft.upstream_model} onChange={e => set({ upstream_model: e.target.value })} />
            </label>}
          </div>
          {formError && <p className="vf-error vf-full" role="alert">{formError}</p>}
          <button className="vf-btn vf-full" type="submit">开始检测</button>
        </div>
      </form>
      <div className="vf-notes"><span>Key 用完即删</span><span>不保存对话内容</span><span>不公开渠道域名</span></div>

      <section className="vf-sec">
        <div className="vf-sec-h"><h2><i>01</i>支持检测的模型</h2><small>{models.data ? `${models.data.models.length} MODELS · ${groups.length} VENDORS` : ''}</small></div>
        <details className="vf-models">
          <summary>
            <div className="vf-vrow">
              {groups.map(([family, list]) => <span key={family} style={{ '--c': vendorOf(family).color } as CSSProperties}>{vendorOf(family).name}<em>{list.length}</em></span>)}
              {!groups.some(([f]) => f.toLowerCase() === 'google') && <span className="vf-pending" style={{ '--c': '#5a6e73' } as CSSProperties}>Gemini<em>采集中</em></span>}
            </div>
            <span className="vf-toggle"><span className="o">查看全部模型</span><span className="c">收起</span></span>
          </summary>
          <div className="vf-vendors" style={{ '--cols': Math.max(groups.length, 1) } as CSSProperties}>
            {groups.map(([family, list]) => <div key={family} style={{ '--c': vendorOf(family).color } as CSSProperties}>
              <h3>{vendorOf(family).name} · {vendorOf(family).series}</h3>
              <ul>{list.map(m => <li key={m.model}>{m.model}</li>)}</ul>
            </div>)}
          </div>
          <div className="vf-mfoot">基准均从官方 API 采集。渠道里的模型名和申报的不一样也能检测。</div>
        </details>
      </section>

      <section className="vf-sec">
        <div className="vf-sec-h"><h2><i>02</i>常见问题</h2><small>FAQ</small></div>
        <div className="vf-faq">{faqs.map(([q, a]) => <details key={q}><summary>{q}</summary><p>{a}</p></details>)}</div>
      </section>
    </main>
    <VerifyFooter />
    {confirming && <ConfirmDialog draft={draft} estimate={chosen?.estimated_cost_usd ?? null} samples={models.data?.target_samples ?? 16}
      onClose={() => setConfirming(false)}
      onStarted={(token, manage) => { localStorage.setItem(manageKey(token), manage); navigate(`/verify/r/${token}`) }} />}
  </div>
}

function ConfirmDialog({ draft, estimate, samples, onClose, onStarted }: { draft: Draft; estimate: number | null; samples: number; onClose: () => void; onStarted: (token: string, manage: string) => void }) {
  const [email, setEmail] = useState('')
  const [code, setCode] = useState('')
  const [requestId, setRequestId] = useState('')
  const [cooldown, setCooldown] = useState(0)
  const [risks, setRisks] = useState([false, false, false])
  const [error, setError] = useState('')

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose() }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])
  useEffect(() => {
    if (cooldown <= 0) return
    const timer = setTimeout(() => setCooldown(c => c - 1), 1000)
    return () => clearTimeout(timer)
  }, [cooldown])

  const sendCode = useMutation({
    mutationFn: () => api<{ request_id: string; retry_after: number }>('/public/verify/email-code', jsonBody({ email: email.trim() })),
    onSuccess: data => { setRequestId(data.request_id); setCooldown(data.retry_after); setError('') },
    onError: e => setError(errorText(e)),
  })
  const start = useMutation({
    mutationFn: () => api<{ token: string; manage_token: string }>('/public/verify', jsonBody({
      protocol: draft.protocol,
      base_url: draft.base_url.trim(),
      api_key: draft.api_key.trim(),
      model: draft.model,
      upstream_model: draft.upstream_model.trim() || null,
      email: email.trim(),
      request_id: requestId,
      code: code.trim(),
      accept_risks: true,
    })),
    onSuccess: data => onStarted(data.token, data.manage_token),
    onError: e => setError(errorText(e)),
  })
  const ready = requestId && /^\d{6}$/.test(code.trim()) && risks.every(Boolean) && !start.isPending
  const riskText = ['费用由这把 Key 承担', '检测请求可能被渠道识别，个别渠道可能因此限制这把 Key', '结论只代表本次抽样']

  return <div className="vf-modal-wrap" role="presentation" onMouseDown={e => { if (e.target === e.currentTarget) onClose() }}>
    <div className="vf-modal" role="dialog" aria-modal="true" aria-labelledby="vf-confirm-title"><div>
      <h3 id="vf-confirm-title">确认开始检测</h3>
      <p className="vf-modal-sub">检测会先发一个很短的请求确认能连通，然后在后台运行。保存报告链接即可随时查看。</p>
      <dl className="vf-sum">
        <dt>接口地址</dt><dd>{draft.base_url.trim()}</dd>
        <dt>申报模型</dt><dd>{draft.model}{draft.upstream_model.trim() && ` · 渠道名 ${draft.upstream_model.trim()}`}</dd>
        <dt>样本数</dt><dd>最多 {samples} 个，可能提前结束</dd>
        <dt>预估花费</dt><dd>{estimate == null ? '暂无该模型单价' : `${usd(estimate)}（按官方单价估算）`}</dd>
      </dl>
      <label className="vf-field">
        <span className="vf-label">邮箱<small>用于验证，不会公开</small></span>
        <div className="vf-code">
          <input className="vf-input" type="email" autoComplete="email" placeholder="you@example.com" value={email} onChange={e => setEmail(e.target.value)} />
          <button type="button" className="vf-send" disabled={!email.includes('@') || cooldown > 0 || sendCode.isPending} onClick={() => sendCode.mutate()}>
            {sendCode.isPending ? '发送中…' : cooldown > 0 ? `${cooldown} 秒后重发` : requestId ? '重新发送' : '发送验证码'}
          </button>
        </div>
      </label>
      <input className="vf-input mono vf-code-input" inputMode="numeric" maxLength={6} placeholder="6 位验证码" aria-label="验证码" value={code} onChange={e => setCode(e.target.value.replace(/\D/g, ''))} />
      {riskText.map((text, i) => <label key={text} className="vf-chk">
        <input type="checkbox" checked={risks[i]} onChange={e => setRisks(r => r.map((v, j) => j === i ? e.target.checked : v))} />{text}
      </label>)}
      {error && <p className="vf-error" role="alert">{error}</p>}
      <div className="vf-modal-actions">
        <button type="button" className="vf-ghost" onClick={onClose}>返回修改</button>
        <button type="button" className="vf-btn" disabled={!ready} onClick={() => start.mutate()}>{start.isPending ? '正在连接渠道…' : '确认，开始检测'}</button>
      </div>
    </div></div>
  </div>
}

// ---------------------------------------------------------------- report page

const verdictCopy: Record<string, { label: string; title: string; color: string }> = {
  match: { label: 'MATCH', title: '与申报模型相符', color: '#80d7a3' },
  family_only: { label: 'FAMILY ONLY', title: '确认厂商，型号难分', color: '#78d9d0' },
  suspected: { label: 'SUSPECTED', title: '疑似不是申报的模型', color: '#e8c37b' },
  confirmed: { label: 'CONFIRMED', title: '确认不是申报的模型', color: '#fb8f9c' },
  ambiguous: { label: 'AMBIGUOUS', title: '结果矛盾', color: '#b9a3f5' },
  insufficient: { label: 'INSUFFICIENT', title: '证据不足', color: '#8fa4a9' },
  unsupported: { label: 'UNSUPPORTED', title: '暂不支持', color: '#8fa4a9' },
}

function explain(report: Report, vendor: string) {
  const closest = report.evidence?.closest_model
  const claimed = report.claimed_model
  switch (report.verdict) {
    case 'match': return <>厂商和型号都和申报的 <span className="mono">{claimed}</span> 一致。一次检测只是一次抽样，不保证渠道从未切换。</>
    case 'family_only': return <>确实是 {vendor} 的模型，但和相邻型号太像，这次分不出来。</>
    case 'suspected': return <>回答更像 <b className="mono">{closest}</b>，和申报的 <span className="mono">{claimed}</span> 不一致。一次检测只是一次抽样，建议持续观察。</>
    case 'confirmed': return <>多项独立证据都指向 <b className="mono">{closest}</b>，渠道很可能没有提供申报的 <span className="mono">{claimed}</span>。</>
    case 'ambiguous': return <>样本之间指向不同模型，渠道可能混用了多个上游或做了协议转换。</>
    case 'insufficient': return <>有效样本太少或证据不一致，本次不下结论。可以稍后再测一次。</>
    default: return <>申报模型暂时没有可用的基准。</>
  }
}

function Shares({ shares, tone }: { shares: Share[]; tone: string }) {
  const top = shares[0]?.model
  return <div className="vf-cmp">
    {shares.map(s => <div key={s.model}>
      <span className="nm">{s.model}{s.claimed ? <em>申报</em> : s.model === top ? <em>最接近</em> : null}</span>
      <span className="vf-bar"><i style={{ width: `${Math.max(s.share * 100, 1)}%`, '--bc': s.model === top ? tone : '#8fa4a9' } as CSSProperties} /></span>
      <span className="pc">{Math.round(s.share * 100)}%</span>
    </div>)}
  </div>
}

const sampleLabels: Record<string, string> = {
  ok: '有效', rate_limited: '被限流', rejected: '被拒绝', server_error: '服务器错误', http_error: '请求错误', network: '网络错误', unusable: '回答不可用',
}

function Evidence({ report }: { report: Report }) {
  const ev = report.evidence!
  const vendor = vendorOf(ev.claimed_family).name
  const rows: { name: string; text: string; pill: string; color: string }[] = [
    ev.vendor.consistent == null
      ? { name: '厂商', text: 'Token 计数样本不足，无法判断厂商', pill: '未判断', color: '#8fa4a9' }
      : ev.vendor.consistent
        ? { name: '厂商', text: `Tokenizer 计数和 ${vendor} 一致`, pill: '一致', color: '#80d7a3' }
        : { name: '厂商', text: `Tokenizer 计数和 ${vendor} 不一致${ev.vendor.families.length ? `，更接近 ${ev.vendor.families.map(f => vendorOf(f).name).join('、')}` : ''}`, pill: '不一致', color: '#fb8f9c' },
    !ev.model.closest
      ? { name: '型号', text: '样本不足，无法比较型号', pill: '未判断', color: '#8fa4a9' }
      : ev.model.same
        ? { name: '型号', text: `回答的统计分布最接近 ${ev.model.closest}`, pill: '一致', color: '#80d7a3' }
        : { name: '型号', text: `回答的统计分布更接近 ${ev.model.closest}`, pill: '不一致', color: '#fb8f9c' },
    ev.tokens?.surplus == null
      ? { name: 'Token 计数', text: '输入 Token 数没有明显多出', pill: '正常', color: '#80d7a3' }
      : { name: 'Token 计数', text: `输入 Token 每次多约 ${ev.tokens!.surplus!.toLocaleString()} 个，可能注入了隐藏提示词`, pill: '异常', color: '#e8c37b' },
  ]
  return <section className="vf-rsec">
    <div className="vf-sec-h"><h2><i>01</i>关键证据</h2><small>EVIDENCE</small></div>
    <div className="vf-evl">{rows.map(r => <div key={r.name} style={{ '--c': r.color } as CSSProperties}><b>{r.name}</b><span>{r.text}</span><span className="vf-pill">{r.pill}</span></div>)}</div>
    <details className="vf-smp">
      <summary><span>样本明细（{ev.samples.length} 条，不含题目和回答）</span><span className="vf-toggle"><span className="o">展开</span><span className="c">收起</span></span></summary>
      <table>
        <thead><tr><th>#</th><th>结果</th><th>耗时</th><th>倾向的模型</th></tr></thead>
        <tbody>{ev.samples.map((s, i) => <tr key={i} className={s.result === 'ok' ? '' : 'bad'}>
          <td className="mono">{i + 1}</td><td>{sampleLabels[s.result] ?? s.result}</td><td className="mono">{(s.latency_ms / 1000).toFixed(1)} s</td><td className="mono">{s.leaning ?? '—'}</td>
        </tr>)}</tbody>
      </table>
    </details>
    <p className="vf-limit">结论基于本次 {report.valid_samples} 个有效样本，渠道可能按时段或负载切换模型。</p>
  </section>
}

export function VerifyReportPage() {
  const { token = '' } = useParams()
  const navigate = useNavigate()
  const [copied, setCopied] = useState(false)
  const [manage] = useState(() => localStorage.getItem(manageKey(token)))
  const report = useQuery({
    queryKey: ['public-verify', token],
    queryFn: ({ signal }) => api<Report>(`/public/verify/${encodeURIComponent(token)}`, { signal, headers: manage ? { 'x-manage-token': manage } : undefined }),
    refetchInterval: q => q.state.data && ['queued', 'running'].includes(q.state.data.status) ? 2000 : false,
    retry: (count, error) => !(error instanceof ApiError && error.status === 404) && count < 3,
  })
  const remove = useMutation({
    mutationFn: () => api<void>(`/public/verify/${encodeURIComponent(token)}`, { method: 'DELETE', headers: { 'x-manage-token': manage ?? '' } }),
    onSuccess: () => { localStorage.removeItem(manageKey(token)); navigate('/verify') },
  })
  const [sharing, setSharing] = useState(false)
  const updateShare = (show_host: boolean) => api<void>(`/public/verify/${encodeURIComponent(token)}`, { ...jsonBody({ show_host }, 'PATCH'), headers: { 'x-manage-token': manage ?? '' } })

  useEffect(() => {
    const meta = document.createElement('meta')
    meta.name = 'robots'
    meta.content = 'noindex'
    document.head.appendChild(meta)
    return () => meta.remove()
  }, [])

  async function copyLink() {
    await copyText(window.location.href).then(() => { setCopied(true); setTimeout(() => setCopied(false), 1600) }, () => undefined)
  }

  // The creator decides whether the shared link shows the channel host
  // before copying it; everyone else copies directly.
  function share() {
    if (report.data?.owner && report.data.host) setSharing(true)
    else void copyLink()
  }

  const data = report.data
  const anonymous = data ? `某 ${protocols.find(p => p.value === data.protocol)?.label ?? ''} 渠道` : ''
  const channel = data?.host ?? anonymous
  const shortId = `#${token.slice(3, 7)}`

  let body
  if (report.isError) {
    const gone = report.error instanceof ApiError && report.error.status === 404
    body = <StateCard color="#8fa4a9" label="NOT FOUND" title={gone ? '报告不存在' : '报告加载失败'} text={gone ? '链接可能有误，或报告已过期、已被删除。' : errorText(report.error)} />
  } else if (!data) {
    body = <StateCard color="#78d9d0" label="LOADING" title="正在加载…" text="" />
  } else if (data.status === 'queued' || data.status === 'running') {
    const queued = data.status === 'queued'
    const failed = data.completed_samples - data.valid_samples
    body = <>
      <div className="vf-crumb"><span>检测中 · {shortId}</span><button type="button" onClick={share}>{copied ? '已复制' : '复制本页链接'}</button></div>
      <div className="vf-vframe" style={{ '--c': '#78d9d0' } as CSSProperties}><div className="vf-vcard">
        <span className="vf-vlabel">{queued ? 'QUEUED' : 'RUNNING'}</span>
        <h1>{queued ? '排队中…' : '正在检测…'}</h1>
        <p className="vf-ex"><span className={data.host ? 'mono' : undefined}>{channel}</span> · 申报 <span className="mono">{data.claimed_model}</span>{queued && data.queue_ahead ? ` · 前面还有 ${data.queue_ahead} 个检测` : ''}</p>
        <div className="vf-ticks" style={{ '--n': data.target_samples } as CSSProperties}>
          {Array.from({ length: data.target_samples }, (_, i) => <i key={i} className={i < data.completed_samples ? 'd' : i === data.completed_samples && !queued ? 'r' : ''} />)}
        </div>
        <div className="vf-prog"><span>已完成 {data.completed_samples} / {data.target_samples} · 有效 {data.valid_samples}{failed > 0 ? ` · 失败 ${failed}` : ''}</span><span>证据足够时会提前结束</span></div>
        {!!data.progress?.shares.length && <div className="vf-lean">
          <div className="t"><span>当前倾向</span><span>证据还在积累，可能变化</span></div>
          <Shares shares={data.progress.shares} tone="linear-gradient(90deg,#78d9d0,#5cc4dc)" />
        </div>}
      </div></div>
      <div className="vf-notes"><span>可以关闭页面，保存链接稍后再看</span></div>
    </>
  } else if (data.status === 'failed') {
    body = <>
      <StateCard color="#fb8f9c" label="FAILED" title="检测没有完成" text={data.error ?? '检测出错，请稍后重试'} />
      <div className="vf-acts"><Link to="/verify">重新检测</Link></div>
    </>
  } else {
    const copy = verdictCopy[data.verdict ?? ''] ?? verdictCopy.insufficient
    const vendor = vendorOf(data.evidence?.claimed_family ?? '').name
    body = <>
      <div className="vf-crumb"><span>检测报告 · {shortId} · {formatDate(data.finished_at ?? data.created_at)}</span><button type="button" onClick={share}>{copied ? '已复制' : '复制分享链接'}</button></div>
      <div className="vf-vframe" style={{ '--c': copy.color } as CSSProperties}><div className="vf-vcard">
        <span className="vf-vlabel">{copy.label}</span>
        <h1>{copy.title}</h1>
        <p className="vf-ex">{explain(data, vendor)}</p>
        {!!data.evidence?.shares.length && <Shares shares={data.evidence.shares} tone={`linear-gradient(90deg,${copy.color},color-mix(in srgb,${copy.color} 70%,white))`} />}
        <div className="vf-meta">
          <div><small>渠道{data.owner && data.host && <em className="vf-scope">{data.show_host ? '已公开' : '仅你可见'}</em>}</small><b className={data.host ? 'mono' : undefined} title={channel}>{channel}</b></div>
          <div><small>申报模型</small><b className="mono">{data.claimed_model}</b></div>
          <div><small>有效样本</small><b className="mono">{data.valid_samples} / {data.completed_samples}</b></div>
          <div><small>本次花费</small><b className="mono">{usd(data.cost_usd)}</b></div>
        </div>
      </div></div>
      {data.evidence && <Evidence report={data} />}
      <div className="vf-next"><div>
        <div><h3>想知道它是不是一直这样？</h3><p>注册后可以给这个渠道开启定时检测，结论变化时提醒你。</p></div>
        <Link className="vf-btn vf-btn-sm" to="/login">持续监控这个渠道 →</Link>
      </div></div>
      <div className="vf-acts">
        <Link to="/verify">再测一次</Link>
        <button type="button" onClick={share}>{copied ? '已复制' : '复制分享链接'}</button>
        {manage && <button type="button" className="danger" disabled={remove.isPending} onClick={() => { if (window.confirm('删除后报告链接将失效，确定删除吗？')) remove.mutate() }}>删除报告</button>}
      </div>
      {remove.isError && <p className="vf-error" role="alert">{errorText(remove.error)}</p>}
    </>
  }

  return <div className="vf-page">
    <Backdrop />
    <SiteHeader />
    <main className="vf-main vf-rmain">{body}</main>
    <VerifyFooter />
    {sharing && data?.host && <ShareDialog host={data.host} anonymous={anonymous} shown={data.show_host}
      onClose={() => setSharing(false)}
      onConfirm={async show => {
        if (show !== data.show_host) { await updateShare(show); await report.refetch() }
        await copyLink()
        setSharing(false)
      }} />}
  </div>
}

function ShareDialog({ host, anonymous, shown, onClose, onConfirm }: { host: string; anonymous: string; shown: boolean; onClose: () => void; onConfirm: (show: boolean) => Promise<void> }) {
  const [show, setShow] = useState(shown)
  const save = useMutation({ mutationFn: () => onConfirm(show) })
  return <div className="vf-modal-wrap" role="presentation" onMouseDown={e => { if (e.target === e.currentTarget) onClose() }}>
    <div className="vf-modal" role="dialog" aria-modal="true" aria-labelledby="vf-share-title"><div>
      <h3 id="vf-share-title">分享这份报告</h3>
      <p className="vf-modal-sub">拿到链接的人都能打开报告。选择别人看到的渠道名称：</p>
      <label className={`vf-opt${show ? '' : ' on'}`}><input type="radio" name="vf-share" checked={!show} onChange={() => setShow(false)} />
        <span><b>隐藏域名</b><small>别人看到“{anonymous}”</small></span></label>
      <label className={`vf-opt${show ? ' on' : ''}`}><input type="radio" name="vf-share" checked={show} onChange={() => setShow(true)} />
        <span><b>显示域名</b><small className="mono">{host}</small></span></label>
      {show && <p className="vf-warn">公开后，报告会和这个渠道直接关联，请确认你愿意公开这份结论。</p>}
      {save.isError && <p className="vf-error" role="alert">{errorText(save.error)}</p>}
      <div className="vf-modal-actions">
        <button type="button" className="vf-ghost" onClick={onClose}>取消</button>
        <button type="button" className="vf-btn" disabled={save.isPending} onClick={() => save.mutate()}>{save.isPending ? '保存中…' : '复制链接'}</button>
      </div>
    </div></div>
  </div>
}

function StateCard({ color, label, title, text }: { color: string; label: string; title: string; text: string }) {
  return <div className="vf-vframe" style={{ '--c': color } as CSSProperties}><div className="vf-vcard">
    <span className="vf-vlabel">{label}</span>
    <h1>{title}</h1>
    {text && <p className="vf-ex">{text}</p>}
  </div></div>
}
