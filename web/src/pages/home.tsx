import { useLayoutEffect, useRef, useState, type CSSProperties } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Check, ChevronDown } from 'lucide-react'
import { Link } from 'react-router-dom'
import { Button } from '../components/ui/button'
import { api, type User } from '../lib/api'
import { AgentsIllustration, FailoverIllustration, ShareIllustration, UsageIllustration, VerifyIllustration } from '../components/home-illustrations'
import { BrandMark, BrandName } from '../components/brand'
import { SiteHeader } from '../components/site-header'
import './home.css'

const agents = ['Claude Code', 'Codex', 'Gemini CLI', 'Qwen Code', 'Kimi CLI', 'DeepSeek Harness', 'ZCode', 'OpenCode']

const highlights = [
  { title: '一键接入', text: 'VAIR Desktop 把渠道里能用的模型写进 Claude Code、Codex 等常用 Agent，用 /model 就能换。' },
  { title: '自动切换', text: '限流、服务出错或 Key 失效，自动换到下一个渠道，Agent 不用改。' },
  { title: '模型验真', text: '持续检查渠道是否换了模型、缩水或虚报用量，发现可疑情况发邮件提醒。' },
  { title: '额度可分享', text: '按人、按设备发 VAIR Key，单独限定模型和费用，上游 Key 不外传。' },
]

const points = {
  agents: [
    { title: '模型跟着渠道走', text: '渠道里加了新模型，点一下同步到本机所有 Agent；换电脑登录后照样能用。' },
    { title: '只列能用的模型', text: 'VAIR 不做协议转换，每个 Agent 只显示与它协议兼容的模型。' },
  ],
  failover: [
    { title: '按你定的顺序换', text: '先用最信任的渠道，出问题再换同协议、同模型的下一家。' },
    { title: '限流先冷却', text: '被限流的渠道暂停一会儿，避免反复重试；Key 被拒绝时直接换下一家。' },
    { title: '不通就提醒', text: '定时检查可用性，渠道不通时暂停路由并发邮件，恢复后自动用回来。' },
  ],
  verify: [
    { title: '真伪检测', text: '比对回答的统计特征和模型基准，证据不够就写“证据不足”。' },
    { title: '服务体检', text: '随机出题，检查回答完整性、工具调用、流式输出和长上下文。' },
    { title: '用量核对', text: '对照真实请求核算 Token，找出虚报和隐藏的系统提示词，不额外发请求。' },
  ],
  sharing: ['每把 Key 单独限定模型、每日和累计费用、并发数', '设备丢了或有人不用了，只撤销那一把', '供应商 Key 加密存在服务端，不下发到设备'],
  usage: ['按 Key、渠道、模型看请求数、Token、费用和速度', '国产模型价格与官方同步', '默认只记录调用元数据，不保存对话内容'],
}

const setupSteps = [
  { title: '添加渠道', text: '填入官方 API 或第三方服务的地址和 Key，也可以添加朋友共享的 VAIR Key。多加几家，互为备份。' },
  { title: '创建 VAIR Key', text: '为自己的设备、同事或应用创建 Key，按需限定可用模型和费用。' },
  { title: '一键接入 Agent', text: '装好 VAIR Desktop，登录后选一把 Key，本机装了的 Agent 会自动配好。' },
]

const audiences = [
  { name: '每天用 Agent 写代码', text: '几台电脑共用一个入口，渠道之间自动互备，模型在 Agent 里随时切换。' },
  { name: '主要用第三方服务', text: '定期检测第三方服务有没有换模型、虚报用量，发现异常发邮件提醒。' },
  { name: '要把额度分给别人', text: '每人一把独立的 Key，限定模型和费用，上游凭据留在自己手里。' },
]

const comparisonOptions = [
  { name: 'VAIR', summary: '在线网关 + 桌面客户端', featured: true },
  { name: 'CC Switch', summary: '桌面工具，切换本机 AI 配置' },
  { name: 'New API', summary: '开源网关，自己找服务器部署' },
  { name: '第三方 API 平台', summary: '购买平台额度，直接调用' },
]

const comparisonRows = [
  { label: '模型额度从哪来', values: ['自己的供应商 Key，或朋友共享的 VAIR Key', '你已有的官方或第三方 Key', '你已有的官方或第三方 Key', '平台提供的模型额度'] },
  { label: '接入 Agent', values: ['一键配置国内外常用 Agent，渠道模型自动同步', '在本机切换供应商配置', '手动填写接口地址和模型', '手动填写接口地址和模型'] },
  { label: '某个渠道出故障', values: ['按优先级自动切换备用渠道，Agent 不用改', '开启本机代理后可自动切换', '可配置多渠道重试', '备用路由由服务平台决定'] },
  { label: '渠道给的是不是真模型', values: ['定时抽样和主动检测，按回答特征判断', '提供端点测速，不检测模型真伪', '提供渠道测试，不检测模型真伪', '依赖平台说明，难以自行验证'] },
  { label: '如何部署使用', values: ['在线使用，无需自建网关', '在电脑上安装使用', '自行部署并维护服务器', '在线使用'] },
  { label: '多设备、多人共用', values: ['按人、设备或应用分配 Key，单独设置权限和额度', '以本机配置管理为主', '支持', '取决于平台'] },
]

function ComparisonCell({ value, featured }: { value: string, featured?: boolean }) {
  return <td className={featured ? 'home-comparison-featured home-comparison-good' : undefined}>
    {featured ? <span className="home-comparison-cell"><Check aria-hidden="true" size={15} /><span>{value}</span></span> : value}
  </td>
}

const faqs = [
  { question: 'VAIR 会提供模型额度吗？', answer: '不会。模型额度来自你接入的渠道，调用费用由供应商向渠道所有者结算。使用他人共享的 VAIR Key 时，上游费用由共享者承担；VAIR 不处理双方之间的收费或结算。' },
  { question: '没有自己的供应商 Key，也能使用吗？', answer: '可以。拿到别人共享的 VAIR Key 后，可以直接粘贴到 VAIR Desktop 里使用，也可以添加为自己帐号的渠道，和已有渠道一起用。可用模型和额度由 Key 的所有者设置，你不需要拿到对方的上游凭据。' },
  { question: 'Agent 怎么接入？', answer: '推荐使用 VAIR Desktop：登录后选择 Key，就能一键配置 Claude Code、Codex、Gemini CLI、Qwen Code、Kimi CLI、DeepSeek Harness、ZCode 和 OpenCode，写入前会备份原配置。其他工具只要支持自定义接口地址，把 Base URL 换成 VAIR 地址、API Key 换成 VAIR Key 即可。' },
  { question: '所有模型都能在任意 Agent 里用吗？', answer: '取决于渠道支持的协议。VAIR 原样透传 OpenAI 兼容、Anthropic Messages 和 Gemini 三种协议，不做协议之间的转换。Agent 使用哪种协议，就能用支持这种协议的渠道里的模型；VAIR Desktop 只会列出当前 Agent 实际能用的模型。' },
  { question: '某个渠道出故障时会怎样？', answer: '遇到限流或暂时故障这类可重试的错误时，VAIR 会按你设置的优先级，把请求转给同协议、同模型的下一个可用渠道，比如从官方 API 切到第三方服务。Agent 这边不用做任何改动。' },
  { question: '每个人、每台设备都要单独配 Key 吗？', answer: '不强制。自己用时，所有设备用同一把 VAIR Key 就行。需要分开管理时，再按人、设备或用途分别创建：每把 Key 可以单独限制可用模型和用量，某台设备丢了或某个人不再使用，只撤销对应的那一把。' },
  { question: '费用限额如何计算？', answer: '费用按平台配置的模型价格和已上报用量估算，可按 Key 设置每日或累计限额。达到限额后暂停新请求，不预扣进行中请求的费用，因此不保证最终金额严格低于上限；估算费用可能与供应商账单不同。' },
  { question: '供应商 Key 放在 VAIR 安全吗？', answer: '供应商 Key 加密保存在服务端，只在服务端访问上游时使用，不会下发到任何设备。设备上只保存 VAIR Key；万一泄露，单独撤销这一把即可，无需更换上游凭据。' },
  { question: '真伪检测的结论可靠吗？', answer: '检测不看模型自报的身份，而是比较回答的统计特征与各候选模型的基准。证据不够时，结论是“证据不足”，不会硬下判断；判定“不符”需要比判定“相符”更强的证据。目前仅支持已采集基准的模型。' },
  { question: '监控和检测会消耗我的额度吗？', answer: '可用性检查、服务体检和真伪检测都用渠道自己的 Key 发出请求，费用计入该渠道的用量。定时抽样每轮 1–8 个样本，频率可在 1–24 小时之间设置；主动检测发起前会显示样本数和预估费用。用量核对只分析真实请求的计量数据，不额外发请求。' },
  { question: 'VAIR 会保存我的对话内容吗？', answer: '默认不保存。VAIR 只记录模型、状态、耗时、Token 用量这类元数据，用于用量统计、计量核对和排查问题。请求与响应的正文不会被保存。' },
]

function PointList({ items }: { items: { title: string; text: string }[] }) {
  return <dl className="home-points">{items.map(({ title, text }) => <div key={title}><dt>{title}</dt><dd>{text}</dd></div>)}</dl>
}

function Brand() {
  return <Link className="brand home-brand" to="/" aria-label="VAIR 首页">
    <BrandMark />
    <BrandName />
  </Link>
}

type Orbit = { path: string, nodes: { x: number, y: number, delay: number }[] }

const orbitPeriod = 4
const orbitStart = 1.4

// The four advantages sit on an arc that curves round the headline. The arc is
// measured from the laid-out items so it stays aligned at any width; below the
// two-column breakpoint (--orbit: 0) the list falls back to a plain stack.
function Highlights() {
  const ref = useRef<HTMLDListElement>(null)
  const [orbit, setOrbit] = useState<Orbit | null>(null)
  useLayoutEffect(() => {
    const list = ref.current
    if (!list) return
    const layout = () => {
      const items = [...list.querySelectorAll<HTMLElement>(':scope > div')]
      if (getComputedStyle(list).getPropertyValue('--orbit').trim() !== '1') {
        items.forEach(item => item.style.removeProperty('--orbit-x'))
        setOrbit(null)
        return
      }
      const ys = items.map(item => item.offsetTop + (item.querySelector('dt')?.offsetHeight ?? 0) / 2)
      const span = ys[ys.length - 1] - ys[0], cy = ys[0] + span / 2, r = span * .95
      const base = Math.sqrt(r * r - (span / 2) ** 2)
      const x = (y: number) => Math.sqrt(r * r - (y - cy) ** 2) - base
      const angle = (y: number) => Math.asin((y - cy) / r)
      const top = ys[0] - span * .2, bottom = ys[ys.length - 1] + span * .2
      const sweep = angle(bottom) - angle(top)
      items.forEach((item, index) => item.style.setProperty('--orbit-x', `${x(ys[index])}px`))
      const nodes = ys.map(y => ({ x: x(y) + 6, y, delay: orbitStart + (angle(y) - angle(top)) / sweep * orbitPeriod }))
      setOrbit({ path: `M${x(top) + 6} ${top} A${r} ${r} 0 0 1 ${x(bottom) + 6} ${bottom}`, nodes })
    }
    layout()
    const observer = new ResizeObserver(layout)
    observer.observe(list)
    return () => observer.disconnect()
  }, [])
  return <dl ref={ref} className="home-highlights">
    {orbit && <svg className="home-orbit" width="1" height="1" aria-hidden="true">
      <defs>
        <linearGradient id="home-orbit-fade" x1="0" y1="0" x2="0" y2="1" gradientUnits="objectBoundingBox">
          <stop offset="0" stopColor="#78d9d0" stopOpacity="0" /><stop offset=".5" stopColor="#78d9d0" /><stop offset="1" stopColor="#78d9d0" stopOpacity="0" />
        </linearGradient>
      </defs>
      <path d={orbit.path} pathLength={1} className="home-orbit-arc" />
      {orbit.nodes.map((node, index) => <g key={node.y} className="home-orbit-stop" style={{ '--i': index } as CSSProperties}>
        <circle cx={node.x} cy={node.y} r="5" className="home-orbit-ring" style={{ animationDelay: `${node.delay}s` }} />
        <circle cx={node.x} cy={node.y} r="4.5" className="home-orbit-node" />
      </g>)}
      <circle r="3" className="home-orbit-spark" opacity="0">
        <animateMotion path={orbit.path} dur={`${orbitPeriod}s`} begin={`${orbitStart}s`} repeatCount="indefinite" />
        <animate attributeName="opacity" values="0;1;1;0" keyTimes="0;.12;.88;1" dur={`${orbitPeriod}s`} begin={`${orbitStart}s`} repeatCount="indefinite" />
      </circle>
    </svg>}
    {highlights.map(({ title, text }, index) => <div key={title} style={{ '--i': index } as CSSProperties}><dt>{title}</dt><dd>{text}</dd></div>)}
  </dl>
}

export function HomePage() {
  const me = useQuery({ queryKey: ['me'], queryFn: ({ signal }) => api<User>('/me', { signal }), retry: false })
  const signedIn = me.isSuccess
  const startTo = signedIn ? '/overview' : '/login'
  const startLabel = signedIn ? '进入控制台' : '开始配置'
  const actions = <div className="home-actions">
    <Button asChild size="lg" className="home-primary-action"><Link to={startTo}>{startLabel}</Link></Button>
    <Link className="home-text-link" to="/download">下载 VAIR Desktop</Link>
  </div>
  return <div className="home-page">
    <a className="skip-link" href="#home-content">跳到主要内容</a>
    <SiteHeader />

    <main id="home-content" tabIndex={-1}>
      <section className="home-hero home-container" aria-labelledby="home-title">
        <div className="home-hero-side">
          <h1 id="home-title">各路 AI 模型，<br />一起接进你的 Agent</h1>
          <p><span className="home-clause">官方 API 和第三方服务放在一个入口里，互为备份。</span><span className="home-clause">用你自己的 Key，无需自建网关。</span></p>
          {actions}
        </div>
        <Highlights />
      </section>

      <section className="home-section home-agents home-container" id="agents" aria-labelledby="agents-title">
        <div className="home-agents-copy">
          <h2 id="agents-title">配置一次，模型随你切换</h2>
          <p className="home-lead">装好 VAIR Desktop，登录后选一把 Key，本机的 Agent 就配好了。渠道里能用的模型都会出现在 Agent 自己的模型列表里，用 /model 就能换。</p>
          <ul className="home-agent-names" aria-label="支持一键接入的 Agent">{agents.map(name => <li key={name}>{name}</li>)}</ul>
        </div>
        <AgentsIllustration />
        <PointList items={points.agents} />
      </section>

      <section className="home-section home-split home-container" id="capabilities" aria-labelledby="capabilities-title">
        <div>
          <h2 id="capabilities-title">一个渠道出问题，工作不会停</h2>
          <p className="home-lead">把官方 API、第三方服务和朋友共享的 Key 都加进来。遇到限流、服务异常或 Key 失效，VAIR 按你定的顺序换下一个渠道，Agent 这边什么都不用改。</p>
          <PointList items={points.failover} />
        </div>
        <FailoverIllustration />
      </section>

      <section className="home-band" id="monitoring" aria-labelledby="monitoring-title">
        <div className="home-split home-split-reverse home-container">
          <div>
            <h2 id="monitoring-title">是不是真模型，用证据说话</h2>
            <p className="home-lead">第三方说给的是旗舰模型，实际是不是？VAIR 不看模型怎么自称，只分析回答本身。</p>
            <PointList items={points.verify} />
            <p className="home-verify-link"><span className="home-clause">还没注册也能先测一家：</span><span className="home-clause"><Link to="/verify">免注册检测渠道</Link>，很快就能拿到一份可以分享的报告。</span></p>
          </div>
          <VerifyIllustration />
        </div>
      </section>

      <section className="home-section home-pair home-container" aria-label="共享额度与用量统计">
        <article id="sharing" aria-labelledby="sharing-title">
          <ShareIllustration />
          <h2 id="sharing-title">额度可以分出去，上游 Key 留在手里</h2>
          <p>给同事、学生或朋友发一把 VAIR Key，能用哪些模型、每天花多少，由你决定。</p>
          <ul>{points.sharing.map(point => <li key={point}>{point}</li>)}</ul>
        </article>
        <article id="usage" aria-labelledby="usage-title">
          <UsageIllustration />
          <h2 id="usage-title">每一笔都查得到</h2>
          <p>每次请求花了多少、走的哪个渠道、响应多快，都有记录。</p>
          <ul>{points.usage.map(point => <li key={point}>{point}</li>)}</ul>
        </article>
      </section>

      <section className="home-section home-audiences home-container" id="audiences" aria-labelledby="audiences-title">
        <h2 id="audiences-title">谁在用 VAIR</h2>
        <dl>{audiences.map(({ name, text }) => <div key={name}><dt>{name}</dt><dd>{text}</dd></div>)}</dl>
      </section>

      <section className="home-comparison-section" id="comparison" aria-labelledby="comparison-title">
        <div className="home-container">
          <h2 id="comparison-title">和其他接入方式比</h2>
          <div className="home-comparison-table" tabIndex={0} role="region" aria-label="接入方式对比表，可横向滚动">
            <table>
              <thead><tr><td />{comparisonOptions.map(({ name, summary, featured }) => <th scope="col" key={name} className={featured ? 'home-comparison-featured' : undefined}>{name}<span>{summary}</span></th>)}</tr></thead>
              <tbody>{comparisonRows.map(({ label, values }) => <tr key={label}><th scope="row">{label}</th>{values.map((value, index) => <ComparisonCell key={comparisonOptions[index].name} value={value} featured={comparisonOptions[index].featured} />)}</tr>)}</tbody>
            </table>
          </div>
        </div>
      </section>

      <section className="home-setup-section" id="getting-started" aria-labelledby="setup-title">
        <div className="home-container">
          <h2 id="setup-title">三步开始用</h2>
          <div className="home-setup-layout">
            <div className="home-setup-side">
              <ol className="home-setup-steps">{setupSteps.map((step, index) => <li key={step.title}><span>{index + 1}</span><div><h3>{step.title}</h3><p>{step.text}</p></div></li>)}</ol>
              <div className="home-actions"><Button asChild><Link to="/download">下载 VAIR Desktop</Link></Button></div>
              <p className="home-manual">不装客户端也可以按文档手动设置：<Link to="/docs/claude-code">Claude Code</Link>、<Link to="/docs/codex">Codex</Link>、<Link to="/docs/other-clients">其他客户端</Link>。</p>
            </div>
            <figure className="home-desktop-shot">
              <img src="/docs-img/desktop-agents-home.webp" width={1920} height={1240} loading="lazy" alt="VAIR Desktop 的本机 Agent 页：六个 Agent 已接入 VAIR，转到中间的是 Claude Code，上下是 Codex 和 DeepSeek Harness，每个都显示所用的模型，可以测试连通或更换 Key。" />
              <figcaption>VAIR Desktop 会找出本机装了的 Agent，选好 Key 和模型就写入配置，写之前先备份，写完自动测一次连通。</figcaption>
            </figure>
          </div>
        </div>
      </section>

      <section className="home-faq home-container" id="questions" aria-labelledby="questions-title">
        <h2 id="questions-title">常见问题</h2>
        <div>{faqs.map(({ question, answer }) => <details key={question}><summary>{question}<ChevronDown aria-hidden="true" size={17} /></summary><p>{answer}</p></details>)}</div>
      </section>

      <section className="home-final-cta home-container" aria-labelledby="cta-title">
        <h2 id="cta-title">先接入你的第一个渠道</h2>
        <p>用自己的供应商 Key，或者朋友共享的 VAIR Key，都可以开始。</p>
        {actions}
      </section>
    </main>

    <footer className="home-footer home-container"><Brand /><nav aria-label="页脚"><Link to="/docs">使用文档</Link><Link to="/verify">模型验证</Link><a href="#questions">常见问题</a></nav><span>© {new Date().getFullYear()} VAIR</span></footer>
  </div>
}
