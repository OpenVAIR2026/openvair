import { ArrowRight, Boxes, Cpu, Timer } from 'lucide-react'
import { Link } from 'react-router-dom'
import { Command } from '../components/copy'
import { useTitle } from '../components/layout'
import { Markdown, Toc, useHashScroll, useToc } from '../components/markdown'
import { content, INSTALL } from '../content'
import { useLang } from '../i18n'

const factIcons = [Timer, Boxes, Cpu]

/** The guide itself is docs/self-hosting.md (or .en.md), the same file people read on GitHub. */
export function DeployPage() {
  const { lang, t: { deploy }, to } = useLang()
  useTitle(deploy.title)
  useHashScroll(`deploy/${lang}`)
  const toc = useToc(`deploy/${lang}`)
  return <div className="page">
    <section className="page-hero">
      <p className="kicker">{deploy.kicker}</p>
      <h1>{deploy.heading}</h1>
      <p className="page-lead">{deploy.lead}</p>
      <Command text={INSTALL} />
      <ul className="facts">{deploy.facts.map(([label, body], index) => {
        const Icon = factIcons[index]
        return <li key={label}><Icon aria-hidden="true" /><strong>{label}</strong><span>{body}</span></li>
      })}</ul>
    </section>
    <div className="doc-layout doc-layout-wide">
      <article className="md" key={lang}>
        <Markdown body={content[lang].deployDoc} />
        <aside className="next-step">
          <p>{deploy.nextTitle}</p>
          <Link to={to('/docs/quickstart')}>{deploy.next}<ArrowRight aria-hidden="true" /></Link>
        </aside>
      </article>
      <Toc {...toc} />
    </div>
  </div>
}
