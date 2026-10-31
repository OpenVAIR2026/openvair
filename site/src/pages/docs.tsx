import { useEffect, useState } from 'react'
import { ArrowLeft, ArrowRight, ChevronDown } from 'lucide-react'
import { Link, useParams } from 'react-router-dom'
import { useTitle } from '../components/layout'
import { Markdown, Toc, useHashScroll, useToc } from '../components/markdown'
import { content } from '../content'
import { useLang } from '../i18n'


function useDocPath() {
  const { to } = useLang()
  return (slug: string) => to(slug ? `/docs/${slug}` : '/docs')
}

function Sidebar({ current }: { current: string }) {
  const { lang, t: { docs } } = useLang()
  const docPath = useDocPath()
  const [open, setOpen] = useState(false)
  useEffect(() => setOpen(false), [current])
  return <aside className="doc-sidebar" data-open={open || undefined}>
    <button type="button" className="doc-sidebar-toggle" aria-expanded={open} onClick={() => setOpen(value => !value)}>{docs.sidebar}<ChevronDown aria-hidden="true" /></button>
    <nav aria-label={docs.sidebar}>
      <Link to={docPath('')} className="doc-sidebar-home" aria-current={current === '' ? 'page' : undefined}>{docs.home}</Link>
      {content[lang].docGroups.map(({ group, pages }) => <div key={group}>
        <p>{group}</p>
        <ul>{pages.map(page => <li key={page.slug}><Link to={docPath(page.slug)} aria-current={page.slug === current ? 'page' : undefined}>{page.title}</Link></li>)}</ul>
      </div>)}
    </nav>
  </aside>
}

function DocsHome() {
  const { lang, t: { docs }, to } = useLang()
  const docPath = useDocPath()
  return <article className="docs-home">
    <p className="kicker">{docs.kicker}</p>
    <h1>{docs.heading}</h1>
    <p className="page-lead">{docs.lead[0]}<Link to={to('/deploy')}>{docs.lead[1]}</Link>{docs.lead[2]}<Link to={docPath('quickstart')}>{docs.lead[3]}</Link>{docs.lead[4]}</p>
    <ul className="doc-cards">{content[lang].docGroups.map(({ group, pages }) => <li key={group}>
      <h2>{group}</h2>
      <ul>{pages.map(page => <li key={page.slug}><Link to={docPath(page.slug)}><strong>{page.title}</strong><span>{page.description}</span></Link></li>)}</ul>
    </li>)}</ul>
  </article>
}

export function DocsPage({ missing = false }: { missing?: boolean }) {
  const param = useParams()['*']?.replace(/\/$/, '') ?? ''
  const slug = missing ? '\0' : param
  const { lang, t: { docs } } = useLang()
  const docPath = useDocPath()
  const { docPages } = content[lang]
  const position = docPages.findIndex(page => page.slug === slug)
  const page = docPages[position]
  useTitle(page ? page.title : slug === '' ? docs.title : docs.missingTitle)
  useHashScroll(`${lang}/${slug}`)
  const toc = useToc(`${lang}/${slug}`)
  const previous = docPages[position - 1]
  const next = docPages[position + 1]

  return <div className="page doc-layout">
    <Sidebar current={slug} />
    {slug === '' ? <DocsHome /> : page ? <>
      <div className="doc-main">
        <article className="md" key={`${lang}/${slug}`}>
          <p className="kicker">{page.group}</p>
          <h1>{page.title}</h1>
          <p className="page-lead">{page.description}</p>
          <Markdown body={page.body} base={`web/src/docs/content/${lang === 'en' ? 'en/' : ''}`} />
        </article>
        <nav className="pager" aria-label={docs.pager}>
          {previous ? <Link to={docPath(previous.slug)}><span><ArrowLeft aria-hidden="true" />{docs.previous}</span><strong>{previous.title}</strong></Link> : <span />}
          {next && <Link to={docPath(next.slug)} className="pager-next"><span>{docs.next}<ArrowRight aria-hidden="true" /></span><strong>{next.title}</strong></Link>}
        </nav>
      </div>
      <Toc {...toc} />
    </> : <article className="docs-home missing">
      <p className="missing-code" aria-hidden="true">404</p>
      <h1>{docs.missingTitle}</h1>
      <p className="page-lead">{docs.missing}</p>
      <p><Link to={docPath('')}>{docs.back}</Link></p>
    </article>}
  </div>
}
