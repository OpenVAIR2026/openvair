import { useMemo } from 'react'
import { ArrowRight, Download, Laptop, Monitor, Terminal } from 'lucide-react'
import { Link } from 'react-router-dom'
import { GithubIcon, useTitle } from '../components/layout'
import { Markdown } from '../components/markdown'
import { content, GITHUB, RELEASES, sectionOf } from '../content'
import { useLang } from '../i18n'

const platforms = [
  { id: 'mac', icon: Laptop, name: 'macOS' },
  { id: 'windows', icon: Monitor, name: 'Windows' },
  { id: 'linux', icon: Terminal, name: 'Linux' },
] as const

const agents = ['Claude Code', 'Codex', 'Gemini CLI', 'Qwen Code', 'Kimi CLI', 'OpenCode', 'ZCode', 'DeepSeek Harness']

function detectPlatform() {
  const agent = navigator.userAgent
  if (/Mac/i.test(agent)) return 'mac'
  if (/Win/i.test(agent)) return 'windows'
  if (/Linux|X11/i.test(agent)) return 'linux'
  return ''
}

export function DownloadPage() {
  const { lang, t: { download }, to } = useLang()
  useTitle(download.title)
  const current = useMemo(detectPlatform, [])
  const desktop = content[lang].docPages.find(page => page.slug === 'desktop')!
  const install = useMemo(() => sectionOf(desktop.body, download.installSection).replace(/^.*GitHub Releases.*\n+/m, ''), [desktop, download])
  const mine = platforms.find(platform => platform.id === current)

  return <div className="page">
    <section className="page-hero download-hero">
      <div>
        <p className="kicker">OpenVAIR Desktop</p>
        <h1><span>{download.heading[0]}</span><br /><span>{download.heading[1]}</span></h1>
        <p className="page-lead">{download.lead}</p>
        <div className="hero-actions">
          <a href={RELEASES} className="button button-primary" target="_blank" rel="noreferrer"><Download aria-hidden="true" />{mine ? download.get(mine.name) : download.latest}</a>
          <a href={`${GITHUB}/tree/main/client`} className="button button-ghost" target="_blank" rel="noreferrer"><GithubIcon />{download.source}</a>
        </div>
        <ul className="agent-chips" aria-label={download.agents}>{agents.map(agent => <li key={agent}>{agent}</li>)}</ul>
      </div>
      <figure className="hero-window download-shot">
        <div className="window-bar" aria-hidden="true"><i /><i /><i /><span>OpenVAIR Desktop</span></div>
        <img src="/docs-img/desktop-agents-home.webp" alt={download.shotAlt} />
      </figure>
    </section>

    <section className="band band-tight">
      <ul className="platforms">{platforms.map(({ id, icon: Icon, name }) => <li key={id} data-current={id === current || undefined}>
        <a href={RELEASES} target="_blank" rel="noreferrer">
          <Icon aria-hidden="true" />
          <strong>{name}</strong>
          <span>{download.files[id]}</span>
          {id === current && <em>{download.current}</em>}
        </a>
      </li>)}</ul>
      <p className="platform-note">{download.note[0]}<a href={RELEASES} target="_blank" rel="noreferrer">GitHub Releases</a>{download.note[1]}</p>
    </section>

    <section className="band band-tight">
      <div className="doc-layout doc-layout-wide">
        <article className="md">
          <h2>{download.install}</h2>
          <Markdown body={install} base={`web/src/docs/content/${lang === 'en' ? 'en/' : ''}`} />
          <aside className="next-step">
            <p>{download.nextTitle}</p>
            <Link to={to('/docs/desktop')}>{download.next}<ArrowRight aria-hidden="true" /></Link>
          </aside>
        </article>
      </div>
    </section>
  </div>
}
