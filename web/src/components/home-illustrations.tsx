import { useId, type ReactNode } from 'react'

// Line illustrations for the marketing homepage feature sections. Purely decorative; the figures are examples.

function Illustration({ children }: { children: (id: string) => ReactNode }) {
  const id = useId().replace(/:/g, '')
  return <div className="home-illo" aria-hidden="true">
    <svg viewBox="0 0 520 400">
      <defs>
        <linearGradient id={`${id}-fill`} x1="0" y1="0" x2="0" y2="1"><stop offset="0" stopColor="#78d9d0" stopOpacity=".22" /><stop offset="1" stopColor="#78d9d0" stopOpacity=".04" /></linearGradient>
        <linearGradient id={`${id}-solid`} x1="0" y1="0" x2="1" y2="1"><stop offset="0" stopColor="#b1fff1" /><stop offset="1" stopColor="#4fb8c4" /></linearGradient>
      </defs>
      {children(id)}
    </svg>
  </div>
}

const chips = [
  { x: 14, y: 50, width: 132, label: '官方 API', to: 'M80 84 C80 118 190 118 222 146' },
  { x: 190, y: 14, width: 140, label: '第三方 A', to: 'M260 48 V146' },
  { x: 368, y: 50, width: 138, label: 'DeepSeek 官方', to: 'M437 84 C437 118 330 118 298 146' },
]

const models = [
  { name: 'gpt-6-luna', source: '官方 API' },
  { name: 'claude-sonnet-5-5', source: '第三方 A' },
  { name: 'deepseek-v4.1', source: 'DeepSeek' },
]

export function AgentsIllustration() {
  return <Illustration>{id => <>
    {chips.map(chip => <g key={chip.label}>
      <path d={chip.to} className="illo-line illo-accent illo-dash illo-flow" />
      <rect x={chip.x} y={chip.y} width={chip.width} height="34" rx="17" className="illo-surface" />
      <circle cx={chip.x + 20} cy={chip.y + 17} r="4" className="illo-dot" />
      <text x={chip.x + 34} y={chip.y + 22} className="illo-text">{chip.label}</text>
    </g>)}
    <rect x="110" y="146" width="300" height="182" rx="16" className="illo-surface" />
    <circle cx="132" cy="166" r="3.5" className="illo-faint" /><circle cx="144" cy="166" r="3.5" className="illo-faint" /><circle cx="156" cy="166" r="3.5" className="illo-faint" />
    <text x="394" y="170" textAnchor="end" className="illo-small">Codex</text>
    <text x="132" y="202" className="illo-mono illo-accent-text">› /model</text>
    {models.map((model, index) => <g key={model.name}>
      {index === 0 && <rect x="124" y="216" width="272" height="30" rx="8" fill={`url(#${id}-fill)`} className="illo-line illo-accent" />}
      <text x="138" y={236 + index * 34} className={index === 0 ? 'illo-mono illo-strong-text' : 'illo-mono illo-dim-text'}>{model.name}</text>
      <text x="384" y={236 + index * 34} textAnchor="end" className="illo-small">{model.source}</text>
    </g>)}
    <path d="M76 336 H444 L424 356 H96 Z" className="illo-surface" />
    <text x="260" y="388" textAnchor="middle" className="illo-muted">三个渠道的模型，同一个列表里选</text>
  </>}</Illustration>
}

export function FailoverIllustration() {
  return <Illustration>{id => <>
    <path d="M96 200 C170 200 170 112 228 112" className="illo-line" />
    <path d="M292 112 C340 112 352 150 378 176" className="illo-line illo-warn illo-dash" />
    <path d="M96 200 C170 200 170 288 228 288" className="illo-line illo-accent illo-strong" />
    <path d="M292 288 C360 288 360 200 418 200" className="illo-line illo-accent illo-strong illo-dash illo-flow" />
    <circle cx="70" cy="200" r="26" className="illo-surface" /><circle cx="70" cy="200" r="7" className="illo-dot" />
    <text x="70" y="250" textAnchor="middle" className="illo-text">Agent</text>
    <text x="70" y="270" textAnchor="middle" className="illo-small">claude-sonnet-5-5</text>
    <circle cx="260" cy="112" r="44" className="illo-line illo-warn illo-dash" opacity=".45" />
    <circle cx="260" cy="112" r="30" className="illo-surface illo-warn" />
    <path d="M250 102 L270 122 M270 102 L250 122" className="illo-line illo-warn illo-strong" />
    <text x="260" y="40" textAnchor="middle" className="illo-text">官方 API</text>
    <text x="260" y="184" textAnchor="middle" className="illo-warn-text">429 限流，冷却 60 秒</text>
    <circle cx="260" cy="288" r="30" fill={`url(#${id}-fill)`} className="illo-line illo-accent" />
    <path d="M247 289 L256 298 L274 278" className="illo-line illo-accent illo-strong" />
    <text x="260" y="342" textAnchor="middle" className="illo-text">第三方 A</text>
    <text x="260" y="362" textAnchor="middle" className="illo-small">自动接手，首字 0.8s</text>
    <circle cx="450" cy="200" r="32" fill={`url(#${id}-solid)`} />
    <path d="M438 200 H462 M452 190 L462 200 L452 210" className="illo-line illo-ink" />
    <text x="450" y="254" textAnchor="middle" className="illo-text">正常返回</text>
    <text x="450" y="274" textAnchor="middle" className="illo-small">Agent 无感知</text>
  </>}</Illustration>
}

const ridges = [16, 30, 44, 58]
const matches = [{ name: 'claude-haiku-4-5', share: .71 }, { name: 'claude-sonnet-5-5', share: .18 }]

export function VerifyIllustration() {
  return <Illustration>{id => <>
    <clipPath id={`${id}-lens`}><circle cx="330" cy="176" r="70" /></clipPath>
    <rect x="66" y="62" width="260" height="276" rx="20" className="illo-surface" />
    <text x="90" y="96" className="illo-text">第三方 B</text>
    <text x="90" y="118" className="illo-small">申报 claude-sonnet-5-5</text>
    {ridges.map(r => <path key={r} d={`M${250 - r} 228 A${r} ${r} 0 1 1 ${250 + r} 228`} className="illo-line" />)}
    <g clipPath={`url(#${id}-lens)`}>
      <circle cx="330" cy="176" r="70" fill={`url(#${id}-fill)`} />
      {ridges.map(r => <path key={r} d={`M${250 - r} 228 A${r} ${r} 0 1 1 ${250 + r} 228`} className="illo-line illo-accent illo-strong" />)}
    </g>
    {matches.map((match, index) => <g key={match.name}>
      <text x="90" y={256 + index * 30} className="illo-small">{match.name}</text>
      <rect x="90" y={264 + index * 30} width="120" height="5" rx="2.5" className="illo-faint" />
      <rect x="90" y={264 + index * 30} width={120 * match.share} height="5" rx="2.5" fill={`url(#${id}-solid)`} />
      <text x="218" y={270 + index * 30} className="illo-small">{Math.round(match.share * 100)}%</text>
    </g>)}
    <circle cx="330" cy="176" r="70" className="illo-line illo-accent illo-ring" />
    <path d="M381 227 L438 284" className="illo-handle" />
    <rect x="352" y="76" width="104" height="30" rx="15" className="illo-badge" />
    <text x="404" y="96" textAnchor="middle" className="illo-warn-text">疑似不符</text>
    <text x="90" y="372" className="illo-muted">24 个样本中，回答特征更接近 claude-haiku-4-5</text>
  </>}</Illustration>
}

const keys = [
  { y: 92, name: '我的笔记本', quota: '今日 $3.10 / $5', used: .62 },
  { y: 200, name: '实验室同学', quota: '今日 $1.90 / $5', used: .38 },
  { y: 308, name: '旧手机', quota: '已撤销', used: 0, revoked: true },
]

export function ShareIllustration() {
  return <Illustration>{id => <>
    {keys.map(key => <path key={key.y} d={`M190 200 C238 200 238 ${key.y} 276 ${key.y}`} className={key.revoked ? 'illo-line illo-dash' : 'illo-line illo-accent illo-dash illo-flow'} />)}
    <rect x="38" y="110" width="152" height="180" rx="24" fill={`url(#${id}-fill)`} className="illo-line illo-accent" />
    <path d="M90 196 V176 A24 24 0 0 1 138 176 V196" className="illo-line illo-accent illo-strong" />
    <rect x="78" y="194" width="72" height="56" rx="12" fill={`url(#${id}-solid)`} />
    <circle cx="114" cy="216" r="6" className="illo-hole" /><rect x="111" y="218" width="6" height="16" rx="3" className="illo-hole" />
    <text x="114" y="318" textAnchor="middle" className="illo-text">上游 Key</text>
    <text x="114" y="338" textAnchor="middle" className="illo-small">加密保存，不出服务端</text>
    {keys.map(key => <g key={key.y} opacity={key.revoked ? .45 : 1}>
      <circle cx="296" cy={key.y} r="15" className={key.revoked ? 'illo-line illo-strong' : 'illo-line illo-accent illo-strong'} />
      <path d={`M311 ${key.y} H356 M342 ${key.y} V${key.y + 10} M352 ${key.y} V${key.y + 8}`} className={key.revoked ? 'illo-line illo-strong' : 'illo-line illo-accent illo-strong'} />
      <text x="376" y={key.y - 8} className="illo-text">{key.name}</text>
      <rect x="376" y={key.y + 2} width="120" height="6" rx="3" className="illo-faint" />
      {!key.revoked && <rect x="376" y={key.y + 2} width={120 * key.used} height="6" rx="3" fill={`url(#${id}-solid)`} />}
      <text x="376" y={key.y + 28} className="illo-small">{key.quota}</text>
    </g>)}
  </>}</Illustration>
}

const lines = [['gpt-6-luna', '$2.84'], ['claude-sonnet-5-5', '$1.92'], ['deepseek-v4.1', '$0.41'], ['kimi-k3', '$0.18']]
const bars = [30, 46, 26, 58, 42, 72]

export function UsageIllustration() {
  return <Illustration>{id => <>
    <path d="M52 40 H262 V300 L244 288 L226 300 L208 288 L190 300 L172 288 L154 300 L136 288 L118 300 L100 288 L82 300 L64 288 L52 300 Z" className="illo-surface" />
    <text x="72" y="72" className="illo-text">今日账单</text>
    {lines.map(([name, cost], index) => <g key={name}>
      <text x="72" y={108 + index * 28} className="illo-small">{name}</text>
      <text x="244" y={108 + index * 28} textAnchor="end" className="illo-small">{cost}</text>
    </g>)}
    <path d="M72 214 H244" className="illo-line illo-dash" />
    <text x="72" y="244" className="illo-text">合计</text>
    <text x="210" y="244" textAnchor="end" className="illo-text illo-accent-text">$5.35</text>
    <rect x="226" y="222" width="250" height="156" rx="18" className="illo-surface" />
    <text x="248" y="250" className="illo-small">近 6 天费用</text>
    <path d="M248 358 H454" className="illo-line" />
    {bars.map((height, index) => <rect key={index} x={256 + index * 33} y={358 - height} width="18" height={height} rx="4" fill={index === bars.length - 1 ? `url(#${id}-solid)` : `url(#${id}-fill)`} className={index === bars.length - 1 ? undefined : 'illo-line illo-accent'} />)}
    <circle cx="410" cy="96" r="40" className="illo-donut-track" />
    <circle cx="410" cy="96" r="40" className="illo-donut" pathLength="100" strokeDasharray="53 100" transform="rotate(-90 410 96)" />
    <text x="410" y="102" textAnchor="middle" className="illo-text">53%</text>
    <text x="410" y="166" textAnchor="middle" className="illo-small">gpt-6-luna 占比</text>
  </>}</Illustration>
}
