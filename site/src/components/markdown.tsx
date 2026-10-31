import { Children, isValidElement, useEffect, useMemo, useState, type ReactElement, type ReactNode } from 'react'
import ReactMarkdown, { type Components } from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { Check, CircleAlert, Copy, Info, Lightbulb, TriangleAlert } from 'lucide-react'
import { Link, useLocation } from 'react-router-dom'
import { highlight } from '../../../web/src/docs/highlight'
import { GITHUB } from '../content'
import { useLang } from '../i18n'
import { copyText } from './copy'

/** Same rule as the console's docs, so anchors written for one work on the other. */
export const slugify = (text: string) => text.trim().toLowerCase().replace(/\s+/g, '-').replace(/[^\p{L}\p{N}_-]/gu, '')

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

/** The console fills {{origin}} with its own address; on the site a placeholder domain reads naturally. */
const EXAMPLE_ORIGIN = 'https://openvair.example.com'

type CalloutKind = 'note' | 'tip' | 'important' | 'warning' | 'caution'
const calloutIcons: Record<CalloutKind, typeof Info> = { note: Info, tip: Lightbulb, important: CircleAlert, warning: TriangleAlert, caution: TriangleAlert }

interface MdNode { type: string; value?: string; children?: MdNode[]; data?: { hProperties?: Record<string, string> } }

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
  const { t: { copy } } = useLang()
  const [copied, setCopied] = useState(false)
  const code = Children.toArray(children).find(isValidElement) as ReactElement<{ className?: string; children?: ReactNode }> | undefined
  const text = textOf(code?.props.children).replace(/\n$/, '')
  const language = /language-(\w+)/.exec(code?.props.className ?? '')?.[1]
  const title = meta || (language && language !== 'text' ? language : '')
  const highlighted = useMemo(() => highlight(text, language), [text, language])
  return <div className="md-code">
    <div className="md-code-header">
      <span className="md-code-dots" aria-hidden="true"><i /><i /><i /></span>
      <span>{title}</span>
      <button type="button" onClick={() => copyText(text).then(() => { setCopied(true); setTimeout(() => setCopied(false), 1600) })}>{copied ? <Check aria-hidden="true" /> : <Copy aria-hidden="true" />}{copied ? copy.copied : copy.copy}</button>
    </div>
    <pre tabIndex={0}><code>{highlighted}</code></pre>
  </div>
}

/** The deployment guide on GitHub, in either language; `#anchor` suffixes are kept. */
const selfHostingUrl = new RegExp(`^${GITHUB.replace(/[.]/g, '\\.')}/blob/main/docs/self-hosting(?:\\.en)?\\.md`)
const siteRoute = /^\/(docs|deploy|download)?([/#]|$)/
/** Repository docs link console pages by file path so GitHub works; on this site they are /docs routes. */
const consoleDocPath = /\/web\/src\/docs\/content\/(?:en\/)?([a-z-]+)\.md$/

/**
 * The same markdown is read in a deployment's console, on this site and on GitHub, so links are mapped here:
 * site routes stay internal, the deployment guide on GitHub becomes /deploy, relative links resolve
 * against the repository (console doc files become /docs pages), and console-only pages such as /my-key become plain text.
 */
function MdLink({ href = '', children, base }: { href?: string; children?: ReactNode; base: string }) {
  const { t, to } = useLang()
  const guide = selfHostingUrl.exec(href)
  if (guide) return <Link to={`${to('/deploy')}${href.slice(guide[0].length)}`}>{children}</Link>
  if (href.startsWith('#')) return <a href={href}>{children}</a>
  if (/^[a-z]+:/i.test(href)) return <a href={href} target="_blank" rel="noreferrer">{children}</a>
  if (href.startsWith('/')) return siteRoute.test(href) ? <Link to={to(href)}>{children}</Link> : <span className="md-console-link" title={t.md.consoleLink}>{children}</span>
  const url = new URL(href, `${GITHUB}/blob/main/${base}`)
  const consoleDoc = consoleDocPath.exec(url.pathname)
  if (consoleDoc) return <Link to={`${to(`/docs/${consoleDoc[1]}`)}${decodeURIComponent(url.hash)}`}>{children}</Link>
  return <a href={url.href} target="_blank" rel="noreferrer">{children}</a>
}

function Callout({ kind, children }: { kind: CalloutKind; children?: ReactNode }) {
  const Icon = calloutIcons[kind]
  return <aside className="md-callout" data-callout={kind}><p className="md-callout-title"><Icon aria-hidden="true" />{useLang().t.md.callouts[kind]}</p>{children}</aside>
}

export function Markdown({ body, base = 'docs/' }: { body: string; base?: string }) {
  const { t } = useLang()
  const components = useMemo<Components>(() => ({
    p: ({ children, node }) => node?.children.length === 1 && (node.children[0] as { tagName?: string }).tagName === 'img' ? <>{children}</> : <p>{children}</p>,
    img: ({ src, alt }) => <figure className="md-shot"><img src={typeof src === 'string' ? src : undefined} alt={alt} loading="lazy" decoding="async" />{alt && <figcaption>{alt}</figcaption>}</figure>,
    pre: ({ children, node }) => <CodeBlock meta={(node?.children[0] as { data?: { meta?: string } } | undefined)?.data?.meta ?? undefined}>{children}</CodeBlock>,
    ol: ({ children, start }) => <ol start={start} style={start ? { counterReset: `step ${start - 1}` } : undefined}>{children}</ol>,
    table: ({ children }) => <div className="md-table" tabIndex={0} role="region" aria-label={t.md.table}><table>{children}</table></div>,
    a: ({ href, children }) => <MdLink href={href} base={base}>{children}</MdLink>,
    blockquote: ({ children, ...props }) => {
      const kind = (props as { 'data-callout'?: CalloutKind })['data-callout']
      return kind ? <Callout kind={kind}>{children}</Callout> : <blockquote>{children}</blockquote>
    },
  }), [base, t])
  return <ReactMarkdown remarkPlugins={[remarkGfm, remarkCallouts]} rehypePlugins={[rehypeHeadingIds]} components={components}>{body.replaceAll('{{origin}}', EXAMPLE_ORIGIN)}</ReactMarkdown>
}

/** Headings of the rendered article, with the one currently at the top of the viewport. */
export function useToc(key: string) {
  const [items, setItems] = useState<{ id: string; text: string; level: number }[]>([])
  const [active, setActive] = useState('')
  useEffect(() => {
    const headings = [...document.querySelectorAll<HTMLElement>('.md h2, .md h3')]
    setItems(headings.map(heading => ({ id: heading.id, text: heading.textContent ?? '', level: heading.tagName === 'H2' ? 2 : 3 })))
    const update = () => setActive([...headings].reverse().find(heading => heading.getBoundingClientRect().top < 120)?.id ?? headings[0]?.id ?? '')
    update()
    window.addEventListener('scroll', update, { passive: true })
    return () => window.removeEventListener('scroll', update)
  }, [key])
  return { items, active }
}

/** Scrolls to the hash after a route change, or to the top when there is none. */
export function useHashScroll(key: string) {
  const { hash } = useLocation()
  useEffect(() => {
    const target = hash ? document.getElementById(decodeURIComponent(hash.slice(1))) : null
    if (target) target.scrollIntoView()
    else window.scrollTo(0, 0)
  }, [key, hash])
}

export function Toc({ items, active }: ReturnType<typeof useToc>) {
  const { t } = useLang()
  if (items.length < 2) return null
  return <aside className="toc" aria-label={t.md.tocLabel}>
    <p>{t.md.toc}</p>
    <ul>{items.map(item => <li key={item.id} data-level={item.level}><a href={`#${item.id}`} aria-current={item.id === active ? 'location' : undefined}>{item.text}</a></li>)}</ul>
  </aside>
}
