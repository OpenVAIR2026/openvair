import { Children, createContext, isValidElement, useContext, useEffect, useMemo, useRef, useState, type CSSProperties, type PointerEvent, type ReactElement, type ReactNode } from 'react'
import { useQuery } from '@tanstack/react-query'
import ReactMarkdown, { type Components } from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { ArrowLeft, ArrowRight, BookOpen, Check, ChevronDown, CircleAlert, Copy, Info, KeyRound, LifeBuoy, Lightbulb, Menu, Network, Plug, Radar, Rocket, Search, ShieldCheck, TerminalSquare, TriangleAlert, X } from 'lucide-react'
import { Link, useLocation, useNavigate, useParams } from 'react-router-dom'
import { api, type User } from '../lib/api'
import { copyText } from '../lib/clipboard'
import { BrandMark, BrandName } from '../components/brand'
import { CLOUD, PRODUCT } from '../lib/edition'
import { docPath, docs, type DocLang, type DocPage } from '../docs/manifest'
import { docsStrings, useDocsLang } from '../docs/lang'
import { highlight } from '../docs/highlight'
import { useOrigin } from '../lib/queries'
import './docs.css'

/** Heading ids double as anchors, so Chinese text is kept and only separators are normalised. */
const slugify = (text: string) => text.trim().toLowerCase().replace(/\s+/g, '-').replace(/[^\p{L}\p{N}_-]/gu, '')

/** Repeated headings on one page get `-2`, `-3`… so every anchor stays unique. */
function uniqueId(base: string, used: Map<string, number>) {
  const count = used.get(base) ?? 0
  used.set(base, count + 1)
  return count ? `${base}-${count + 1}` : base
}

function textOf(node: ReactNode): string {
  if (typeof node === 'string' || typeof node === 'number') return String(node)
  if (Array.isArray(node)) return node.map(textOf).join('')
  if (isValidElement<{ children?: ReactNode }>(node)) return textOf(node.props.children)
  return ''
}

/** Examples are written with {{origin}} so readers can copy them for the site they are on. */
const fill = (body: string, origin = window.location.origin) => body.replaceAll('{{origin}}', origin)

/** Inline markdown reduced to the text a reader sees; matches what the rendered heading contains. */
const inlinePlain = (text: string) => text.replace(/!?\[([^\]]*)\]\([^)]*\)/g, '$1').replace(/[*`]/g, '')

/** Plain text for search. `_` and `-` are kept so error codes such as `rpm_limit` stay searchable. */
const plain = (text: string) => inlinePlain(text).replace(/^```.*$/gm, ' ').replace(/\[!\w+\]/g, ' ').replace(/[#>|]+/g, ' ').replace(/\s+/g, ' ').trim()

type CalloutKind = 'note' | 'tip' | 'important' | 'warning' | 'caution'
const calloutIcons: Record<CalloutKind, typeof Info> = { note: Info, tip: Lightbulb, important: CircleAlert, warning: TriangleAlert, caution: TriangleAlert }

const groupIcons: Record<string, typeof Info> = { start: Rocket, tools: Plug, channels: Network, keys: KeyRound, monitoring: Radar, account: ShieldCheck, help: LifeBuoy }

/** The reader's docs language, so deeply nested markdown pieces can label themselves without prop drilling. */
const LangContext = createContext<DocLang>('zh')
const useStrings = () => docsStrings[useContext(LangContext)]

interface MdNode { type: string; value?: string; children?: MdNode[]; data?: { hProperties?: Record<string, string> } }

/** Turns GitHub-style alerts (`> [!TIP]`) into callouts by tagging the blockquote. */
function remarkCallouts() {
  const visit = (node: MdNode) => {
    if (node.type === 'blockquote') {
      const first = node.children?.[0]?.children?.[0]
      const match = first?.type === 'text' ? /^\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION)\]\s*/.exec(first.value ?? '') : null
      if (first && match) {
        first.value = first.value!.slice(match[0].length)
        node.data = { hProperties: { 'data-callout': match[1].toLowerCase() } }
      }
    }
    node.children?.forEach(visit)
  }
  return (tree: MdNode) => visit(tree)
}

interface HastNode { type: string; tagName?: string; value?: string; properties?: Record<string, unknown>; children?: HastNode[] }
const hastText = (node: HastNode): string => node.type === 'text' ? node.value ?? '' : (node.children ?? []).map(hastText).join('')

/** Assigns heading ids once per page so duplicates are numbered in document order. */
function rehypeHeadingIds() {
  return (tree: HastNode) => {
    const used = new Map<string, number>()
    const visit = (node: HastNode) => {
      if (node.tagName === 'h2' || node.tagName === 'h3') node.properties = { ...node.properties, id: uniqueId(slugify(hastText(node)), used) }
      node.children?.forEach(visit)
    }
    visit(tree)
  }
}

function CodeBlock({ children, meta }: { children?: ReactNode; meta?: string }) {
  const t = useStrings()
  const [copied, setCopied] = useState<'idle' | 'copied' | 'error'>('idle')
  const code = Children.toArray(children).find(isValidElement) as ReactElement<{ className?: string; children?: ReactNode }> | undefined
  const text = textOf(code?.props.children).replace(/\n$/, '')
  const language = /language-(\w+)/.exec(code?.props.className ?? '')?.[1]
  // The fence info after the language (```bash 终端) names the block; otherwise the language does.
  const title = meta || (language && language !== 'text' ? language : '')
  const highlighted = useMemo(() => highlight(text, language), [text, language])
  async function copy() {
    try { await copyText(text); setCopied('copied'); setTimeout(() => setCopied('idle'), 1600) } catch { setCopied('error') }
  }
  return <div className="docs-code" data-language={language}>
    <div className="docs-code-header">
      <span className="docs-code-dots" aria-hidden="true"><i /><i /><i /></span>
      <span className="docs-code-title">{title}</span>
      <button type="button" onClick={copy} aria-label={copied === 'copied' ? t.copied : t.copyCode}>{copied === 'copied' ? <Check aria-hidden="true" /> : <Copy aria-hidden="true" />}{copied === 'copied' ? t.copied : copied === 'error' ? t.copyFailed : t.copy}</button>
    </div>
    <pre tabIndex={0}><code>{highlighted}</code></pre>
  </div>
}

function Heading({ level, id, children }: { level: 2 | 3; id?: string; children?: ReactNode }) {
  const Tag = `h${level}` as const
  return <Tag id={id}>{children}</Tag>
}

/** Screenshots: a captioned figure that opens full size in a modal dialog. */
function DocImage({ src = '', alt = '' }: { src?: string; alt?: string }) {
  const t = useStrings()
  const dialog = useRef<HTMLDialogElement>(null)
  return <figure className="docs-shot">
    <button type="button" onClick={() => dialog.current?.showModal()} aria-label={`${t.zoom}${alt}`}><img src={src} alt={alt} loading="lazy" decoding="async" /></button>
    {alt && <figcaption>{alt}</figcaption>}
    <dialog ref={dialog} className="docs-shot-zoom" onClick={() => dialog.current?.close()}><img src={src} alt={alt} /></dialog>
  </figure>
}

function DocTable({ children }: { children?: ReactNode }) {
  return <div className="docs-table" tabIndex={0} role="region" aria-label={useStrings().table}><table>{children}</table></div>
}

function Callout({ kind, children }: { kind: CalloutKind; children?: ReactNode }) {
  const Icon = calloutIcons[kind]
  return <aside className="docs-callout" data-callout={kind}><p className="docs-callout-title"><Icon aria-hidden="true" />{useStrings().callouts[kind]}</p>{children}</aside>
}

const markdownComponents: Components = {
  // A paragraph holding only an image becomes the figure itself, since <figure> cannot sit inside <p>.
  p: ({ children, node }) => node?.children.length === 1 && (node.children[0] as { tagName?: string }).tagName === 'img' ? <>{children}</> : <p>{children}</p>,
  img: ({ src, alt }) => <DocImage src={typeof src === 'string' ? src : undefined} alt={alt} />,
  h2: ({ children, id }) => <Heading level={2} id={id}>{children}</Heading>,
  h3: ({ children, id }) => <Heading level={3} id={id}>{children}</Heading>,
  pre: ({ children, node }) => <CodeBlock meta={(node?.children[0] as { data?: { meta?: string } } | undefined)?.data?.meta ?? undefined}>{children}</CodeBlock>,
  // Numbered lists are drawn as steps; a list that resumes after a code block keeps counting.
  ol: ({ children, start }) => <ol start={start} style={start ? { counterReset: `step ${start - 1}` } : undefined}>{children}</ol>,
  table: ({ children }) => <DocTable>{children}</DocTable>,
  a: ({ href = '', children }) => href.startsWith('/') && !href.startsWith('/v1')
    ? <Link to={href}>{children}</Link>
    : href.startsWith('#') ? <a href={href}>{children}</a> : <a href={href} target="_blank" rel="noreferrer">{children}</a>,
  blockquote: ({ children, ...props }) => {
    const kind = (props as { 'data-callout'?: CalloutKind })['data-callout']
    return kind ? <Callout kind={kind}>{children}</Callout> : <blockquote>{children}</blockquote>
  },
}

function useToc(slug: string) {
  const [items, setItems] = useState<{ id: string; text: string; level: number }[]>([])
  const [active, setActive] = useState('')
  useEffect(() => {
    const headings = [...document.querySelectorAll<HTMLElement>('.docs-article h2, .docs-article h3')]
    setItems(headings.map(heading => ({ id: heading.id, text: heading.textContent ?? '', level: heading.tagName === 'H2' ? 2 : 3 })))
    const update = () => setActive([...headings].reverse().find(heading => heading.getBoundingClientRect().top < 120)?.id ?? headings[0]?.id ?? '')
    update()
    window.addEventListener('scroll', update, { passive: true })
    return () => window.removeEventListener('scroll', update)
  }, [slug])
  return { items, active }
}

/** Writes scroll progress straight to the bar so scrolling never re-renders the page. */
function useReadingProgress(slug: string) {
  const bar = useRef<HTMLSpanElement>(null)
  useEffect(() => {
    const update = () => {
      const max = document.documentElement.scrollHeight - window.innerHeight
      bar.current?.style.setProperty('transform', `scaleX(${max > 0 ? Math.min(1, window.scrollY / max) : 0})`)
    }
    update()
    window.addEventListener('scroll', update, { passive: true })
    window.addEventListener('resize', update)
    return () => { window.removeEventListener('scroll', update); window.removeEventListener('resize', update) }
  }, [slug])
  return bar
}

interface Section { id: string; title: string; text: string }

/** Splits a page at its headings so a search hit can jump straight to the matching section. */
function sectionsOf(page: DocPage): Section[] {
  const used = new Map<string, number>()
  const sections = [{ id: '', title: page.title, lines: [] as string[] }]
  let fenced = false
  for (const line of fill(page.body).split('\n')) {
    if (line.startsWith('```')) fenced = !fenced
    const heading = fenced ? null : /^#{2,3}\s+(.+)$/.exec(line)
    if (heading) {
      const title = inlinePlain(heading[1]).trim()
      sections.push({ id: uniqueId(slugify(title), used), title, lines: [] })
    } else sections[sections.length - 1].lines.push(line)
  }
  return sections.map(({ id, title, lines }) => ({ id, title, text: plain(lines.join('\n')) }))
}

function Marked({ text, terms }: { text: string; terms: string[] }) {
  if (!terms.length) return <>{text}</>
  const pattern = new RegExp(`(${terms.map(term => term.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('|')})`, 'gi')
  return <>{text.split(pattern).map((part, index) => index % 2 ? <mark key={index}>{part}</mark> : part)}</>
}

function SearchDialog({ lang, initialQuery, onClose }: { lang: DocLang; initialQuery: string; onClose: () => void }) {
  const t = docsStrings[lang]
  const { docPages } = docs[lang]
  const navigate = useNavigate()
  const [query, setQuery] = useState(initialQuery)
  const [selected, setSelected] = useState(0)
  const list = useRef<HTMLUListElement>(null)
  const index = useMemo(() => docPages.map(page => {
    const sections = sectionsOf(page)
    return { page, sections, haystack: `${page.title} ${page.description} ${sections.map(section => `${section.title} ${section.text}`).join(' ')}`.toLowerCase() }
  }), [docPages])
  const terms = query.trim().toLowerCase().split(/\s+/).filter(Boolean)
  const results = terms.length ? index.flatMap(({ page, sections, haystack }) => {
    if (!terms.every(term => haystack.includes(term))) return []
    const inTitle = (section: Section) => terms.some(term => section.title.toLowerCase().includes(term))
    const inText = (section: Section) => terms.every(term => `${section.title} ${section.text}`.toLowerCase().includes(term))
    const pageHit = terms.some(term => page.title.toLowerCase().includes(term))
    const section = sections.find(candidate => candidate.id && inTitle(candidate)) ?? sections.find(inText) ?? sections.find(candidate => candidate.text.toLowerCase().includes(terms[0]))
    const at = section ? section.text.toLowerCase().indexOf(terms[0]) : -1
    const snippet = !section || at < 0 ? page.description : `${at > 36 ? '…' : ''}${section.text.slice(Math.max(0, at - 36), at + 84)}…`
    return [{ page, section: section?.id ? section : undefined, snippet, score: pageHit ? 0 : section?.id && inTitle(section) ? 1 : 2 }]
  }).sort((a, b) => a.score - b.score).slice(0, 10) : docPages.slice(0, 6).map(page => ({ page, section: undefined as Section | undefined, snippet: page.description, score: 0 }))

  useEffect(() => { list.current?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest' }) }, [selected])

  function open(result: (typeof results)[number]) {
    navigate(`${docPath(result.page.slug)}${result.section ? `#${result.section.id}` : ''}`)
    onClose()
  }
  const optionId = (position: number) => `docs-search-option-${position}`

  return <div className="docs-search-backdrop" onMouseDown={onClose}>
    <div className="docs-search" role="dialog" aria-modal="true" aria-label={t.search} onMouseDown={event => event.stopPropagation()}>
      <label className="docs-search-input"><Search aria-hidden="true" /><input autoFocus role="combobox" aria-expanded="true" aria-autocomplete="list" value={query} placeholder={t.searchPlaceholder} aria-label={t.search} aria-controls="docs-search-results" aria-activedescendant={results.length ? optionId(selected) : undefined} onChange={event => { setQuery(event.target.value); setSelected(0) }} onKeyDown={event => {
        if (event.key === 'Escape') onClose()
        // The input is the only focus stop; results are reached with the arrow keys.
        else if (event.key === 'Tab') event.preventDefault()
        else if (event.key === 'ArrowDown') { event.preventDefault(); setSelected(value => Math.min(value + 1, results.length - 1)) }
        else if (event.key === 'ArrowUp') { event.preventDefault(); setSelected(value => Math.max(value - 1, 0)) }
        else if (event.key === 'Enter' && results[selected]) open(results[selected])
      }} /><kbd>Esc</kbd></label>
      {!terms.length && <p className="docs-search-hint">{t.common}{t.popular.map(term => <button key={term} type="button" onClick={() => { setQuery(term); setSelected(0) }}>{term}</button>)}</p>}
      <ul ref={list} id="docs-search-results" role="listbox" aria-label={t.searchResults}>
        {results.length ? results.map((result, position) => <li key={`${result.page.slug}#${result.section?.id ?? ''}`} id={optionId(position)} role="option" aria-selected={position === selected} onMouseEnter={() => setSelected(position)} onClick={() => open(result)}>
          <span className="docs-search-group">{result.page.group}</span>
          <strong><Marked text={result.page.title} terms={terms} />{result.section && <span className="docs-search-section"> › <Marked text={result.section.title} terms={terms} /></span>}</strong>
          <span className="docs-search-snippet"><Marked text={result.snippet} terms={terms} /></span>
        </li>) : <li className="docs-search-empty">{t.noResults(query.trim())}<Link to="/docs/faq" onClick={onClose}>{t.faq}</Link>{lang === 'en' ? '.' : '。'}</li>}
      </ul>
      <p className="docs-search-footer" aria-hidden="true"><span><kbd>↑</kbd><kbd>↓</kbd> {t.select}</span><span><kbd>Enter</kbd> {t.open}</span><span><kbd>Esc</kbd> {t.close}</span></p>
    </div>
  </div>
}

function Sidebar({ lang, current, onNavigate }: { lang: DocLang; current: string; onNavigate: () => void }) {
  return <nav className="docs-sidebar-nav" aria-label={docsStrings[lang].sidebar}>{docs[lang].docGroups.map(({ id, group, pages }) => {
    const Icon = groupIcons[id] ?? BookOpen
    return <div key={group}>
      <p><Icon aria-hidden="true" />{group}</p>
      <ul>{pages.map(page => <li key={page.slug}><Link to={docPath(page.slug)} aria-current={page.slug === current ? 'page' : undefined} onClick={onNavigate}>{page.title}</Link></li>)}</ul>
    </div>
  })}</nav>
}

/** Illustrates the failover path: the request skips a rate-limited channel and lands on a healthy one. */
function RouteFlow() {
  const { flow } = useStrings()
  return <figure className="docs-flow" aria-label={flow.label(PRODUCT)}>
    <div className="docs-flow-node"><TerminalSquare aria-hidden="true" /><span>{flow.tools}<small>Claude Code · Codex · SDK</small></span></div>
    <span className="docs-flow-wire" aria-hidden="true" />
    <div className="docs-flow-node docs-flow-hub"><BrandMark /><span>{PRODUCT}<small>{flow.hub}</small></span></div>
    <svg className="docs-flow-fork" viewBox="0 0 200 40" preserveAspectRatio="none" aria-hidden="true">
      <path className="docs-flow-path-down" d="M100 0 V16 H50 V40" vectorEffect="non-scaling-stroke" />
      <path className="docs-flow-path-up" d="M100 0 V16 H150 V40" vectorEffect="non-scaling-stroke" />
    </svg>
    <div className="docs-flow-targets">
      <div className="docs-flow-node" data-state="down"><span>{flow.a}<small>{flow.limited}</small></span></div>
      <div className="docs-flow-node" data-state="up"><span>{flow.b}<small>{flow.healthy}</small></span></div>
    </div>
    <figcaption>{flow.caption}</figcaption>
  </figure>
}

function spotlight(event: PointerEvent<HTMLElement>) {
  const card = event.currentTarget
  const box = card.getBoundingClientRect()
  card.style.setProperty('--x', `${event.clientX - box.left}px`)
  card.style.setProperty('--y', `${event.clientY - box.top}px`)
}

function DocsHome({ lang, onSearch }: { lang: DocLang; onSearch: (query?: string) => void }) {
  const t = docsStrings[lang]
  return <>
    <section className="docs-hero">
      <div className="docs-hero-copy">
        <p className="docs-eyebrow">{t.hero.eyebrow(PRODUCT)}</p>
        <h1>{t.hero.title}<em>{t.hero.titleEm}</em></h1>
        <p className="docs-lead">{t.hero.lead}</p>
        <div className="docs-hero-actions">
          <Link to="/docs/quickstart" className="docs-hero-primary">{t.hero.quickstart}<ArrowRight aria-hidden="true" /></Link>
          <button type="button" className="docs-hero-search" onClick={() => onSearch()}><Search aria-hidden="true" /><span>{t.search}</span><kbd>⌘K</kbd></button>
        </div>
        <p className="docs-hero-popular">{t.popularLabel}{t.popular.slice(0, 4).map(term => <button key={term} type="button" onClick={() => onSearch(term)}>{term}</button>)}</p>
      </div>
      <RouteFlow />
    </section>
    <ul className="docs-cards">
      {docs[lang].docGroups.map(({ id, group, pages }, order) => {
        const Icon = groupIcons[id] ?? BookOpen
        const summary = (t.groups[id] ?? '').replace('{product}', PRODUCT)
        return <li key={group} className="docs-card" onPointerMove={spotlight} style={{ '--order': order } as CSSProperties}>
          <span className="docs-card-icon"><Icon aria-hidden="true" /></span>
          <h2>{group}<span>{t.pages(pages.length)}</span></h2>
          <p>{summary}</p>
          <ul>{pages.filter(page => page.slug).slice(0, 4).map(page => <li key={page.slug}><Link to={docPath(page.slug)}>{page.title}<ArrowRight aria-hidden="true" /></Link></li>)}</ul>
        </li>
      })}
    </ul>
  </>
}

export function DocsPage() {
  const slug = useParams()['*']?.replace(/\/$/, '') ?? ''
  const location = useLocation()
  const me = useQuery({ queryKey: ['me'], queryFn: ({ signal }) => api<User>('/me', { signal }), retry: false })
  const [search, setSearch] = useState<string | null>(null)
  const [menuOpen, setMenuOpen] = useState(false)
  const [lang, setLang] = useDocsLang()
  const t = docsStrings[lang]
  const { docPages } = docs[lang]
  const position = docPages.findIndex(page => page.slug === slug)
  const page = docPages[position]
  const { items, active } = useToc(`${lang}/${slug}`)
  const progress = useReadingProgress(slug)
  const openSearch = (query = '') => setSearch(query)

  useEffect(() => { document.title = page ? `${page.title} · ${t.hero.eyebrow(PRODUCT)}` : t.hero.eyebrow(PRODUCT) }, [page, t])
  useEffect(() => {
    const target = location.hash ? document.getElementById(decodeURIComponent(location.hash.slice(1))) : null
    if (target) target.scrollIntoView()
    else window.scrollTo(0, 0)
  }, [slug, location.hash])
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.key === 'k' && (event.metaKey || event.ctrlKey)) || (event.key === '/' && !(event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement))) { event.preventDefault(); setSearch(value => value ?? '') }
      else if (event.key === 'Escape') setMenuOpen(false)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])
  // The page behind an open menu or search dialog should not scroll.
  useEffect(() => {
    if (!menuOpen && search === null) return
    const root = document.documentElement
    const previous = root.style.overflow
    root.style.overflow = 'hidden'
    return () => { root.style.overflow = previous }
  }, [menuOpen, search])

  const previous = docPages[position - 1]
  const next = docPages[position + 1]
  const origin = useOrigin()
  const body = useMemo(() => page ? fill(page.body, origin) : '', [page, origin])
  const isHome = page?.slug === ''

  return <LangContext.Provider value={lang}><div className="docs-page">
    <a className="skip-link" href="#docs-content">{t.skip}</a>
    <header className="docs-header">
      <button type="button" className="docs-menu-button" aria-label={menuOpen ? t.closeMenu : t.openMenu} aria-expanded={menuOpen} onClick={() => setMenuOpen(open => !open)}>{menuOpen ? <X aria-hidden="true" /> : <Menu aria-hidden="true" />}</button>
      <Link to="/" className="brand docs-brand" aria-label={`${PRODUCT} ${t.home}`}><BrandMark /><BrandName /></Link>
      <Link to="/docs" className="docs-header-label">{t.docs}</Link>
      <button type="button" className="docs-search-trigger" onClick={() => openSearch()}><Search aria-hidden="true" /><span>{t.search}</span><kbd>⌘K</kbd></button>
      <div className="docs-header-actions">
        <div className="docs-lang" role="group" aria-label={t.language}>{([['zh', '中文'], ['en', 'EN']] as const).map(([value, label]) => <button key={value} type="button" lang={value === 'en' ? 'en' : 'zh-CN'} aria-pressed={lang === value} onClick={() => setLang(value)}>{label}</button>)}</div>
        {CLOUD && <nav className="docs-site-nav" aria-label={t.siteNav}><Link to="/">{t.home}</Link><Link to="/verify">{t.verify}</Link><Link to="/download">{t.desktop}</Link></nav>}
        {CLOUD && !me.isSuccess && <Link to="/login" className="docs-header-link">{t.login}</Link>}
        <Link to={me.isSuccess ? '/overview' : '/login'} className="docs-header-cta">{me.isSuccess ? t.console : CLOUD ? t.start : t.login}</Link>
      </div>
      <div className="docs-progress" aria-hidden="true"><span ref={progress} /></div>
    </header>
    <div className="docs-layout" data-home={isHome || undefined}>
      <aside className="docs-sidebar" data-open={menuOpen}><Sidebar lang={lang} current={slug} onNavigate={() => setMenuOpen(false)} /></aside>
      <main id="docs-content" tabIndex={-1} className="docs-main">
        {page ? <>
          <article key={`${lang}/${slug}`} className="docs-article">
            {isHome ? <DocsHome lang={lang} onSearch={openSearch} /> : <>
              <p className="docs-eyebrow">{page.group}</p>
              <h1>{page.title}</h1>
              <p className="docs-lead">{page.description}</p>
            </>}
            {!isHome && items.length > 1 && <details className="docs-toc-inline">
              <summary>{t.toc}<ChevronDown aria-hidden="true" /></summary>
              <ul>{items.map(item => <li key={item.id} data-level={item.level}><a href={`#${item.id}`}>{item.text}</a></li>)}</ul>
            </details>}
            <ReactMarkdown remarkPlugins={[remarkGfm, remarkCallouts]} rehypePlugins={[rehypeHeadingIds]} components={markdownComponents}>{body}</ReactMarkdown>
          </article>
          <nav className="docs-pager" aria-label={t.pager}>
            {previous ? <Link to={docPath(previous.slug)}><span><ArrowLeft aria-hidden="true" />{t.previous}</span><strong>{previous.title}</strong></Link> : <span />}
            {next && <Link to={docPath(next.slug)} className="docs-pager-next"><span>{t.next}<ArrowRight aria-hidden="true" /></span><strong>{next.title}</strong></Link>}
          </nav>
          <p className="docs-feedback">{t.feedback[0]}<Link to="/docs/troubleshooting">{t.feedback[1]}</Link>{t.feedback[2]}</p>
        </> : <article className="docs-article docs-missing">
          <p className="docs-missing-code" aria-hidden="true">404</p>
          <h1>{t.missingTitle}</h1>
          <p className="docs-lead">{t.missing[0]}<button type="button" className="docs-inline-button" onClick={() => openSearch()}>{t.missing[1]}</button>{t.missing[2]}</p>
          <p><Link to="/docs">{t.back}</Link></p>
        </article>}
      </main>
      {page && !isHome && items.length > 1 && <aside className="docs-toc" aria-label={t.tocLabel}>
        <p>{t.toc}</p>
        <ul>{items.map(item => <li key={item.id} data-level={item.level}><a href={`#${item.id}`} aria-current={item.id === active ? 'location' : undefined}>{item.text}</a></li>)}</ul>
      </aside>}
    </div>
    {search !== null && <SearchDialog lang={lang} initialQuery={search} onClose={() => setSearch(null)} />}
  </div></LangContext.Provider>
}
