import { useEffect, useId, useState, type ReactNode } from 'react'
import { Menu, X } from 'lucide-react'
import { Link, NavLink, useLocation, useNavigate } from 'react-router-dom'
import { LANG_KEY, langOf, pathIn, strings, useLang, type Lang } from '../i18n'
import { GITHUB } from '../links'

/** The console's mark: three channels converging on one node, the outer strokes forming a V. */
export function BrandMark({ size = 28 }: { size?: number }) {
  const gradient = useId()
  return <svg width={size} height={size} viewBox="2.5 2 19 19" aria-hidden="true" className="brand-mark">
    <defs><linearGradient id={gradient} gradientUnits="userSpaceOnUse" x1="4" y1="4" x2="20" y2="20"><stop offset="0" stopColor="#d6fff8" /><stop offset=".55" stopColor="#78d9d0" /><stop offset="1" stopColor="#3aa8d8" /></linearGradient></defs>
    <path d="M12 6.5V18.5" stroke={`url(#${gradient})`} strokeOpacity=".45" strokeWidth="1.6" strokeLinecap="round" fill="none" />
    <path d="M4.5 5L12 18.5L19.5 5" stroke={`url(#${gradient})`} strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round" fill="none" />
    <circle cx="4.5" cy="5" r="1.7" fill="#d6fff8" />
    <circle cx="12" cy="5" r="1.4" fill="#78d9d0" fillOpacity=".7" />
    <circle cx="19.5" cy="5" r="1.7" fill="#5cc4dc" />
    <circle cx="12" cy="18.5" r="2.3" fill="#ecfffb" />
  </svg>
}

export function GithubIcon() {
  return <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true" fill="currentColor"><path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z" /></svg>
}

function LangSwitch() {
  const { pathname, search } = useLocation()
  const { lang, t } = useLang()
  const remember = (value: Lang) => { try { localStorage.setItem(LANG_KEY, value) } catch { /* the link still switches */ } }
  return <div className="site-lang" role="group" aria-label={t.nav.switchLabel}>
    {(['zh', 'en'] as const).map(value => <Link key={value} to={`${pathIn(value, pathname)}${search}`} lang={strings[value].htmlLang} hrefLang={strings[value].htmlLang} aria-current={value === lang ? 'true' : undefined} onClick={() => remember(value)}>{value === 'zh' ? '中文' : 'EN'}</Link>)}
  </div>
}

/**
 * On a first visit to a Chinese page from a non-Chinese browser, move to the English page once.
 * Whatever language the visitor ends up on is remembered, so a later visit or an explicit
 * choice in the switcher is never overridden. Crawlers are left on the URL they asked for.
 */
function useFirstVisitLang() {
  const navigate = useNavigate()
  const { pathname, search, hash } = useLocation()
  const lang = langOf(pathname)
  useEffect(() => {
    let saved: string | null = null
    try { saved = localStorage.getItem(LANG_KEY) } catch { return }
    if (saved) return
    const next: Lang = lang === 'zh' && !/^zh\b/i.test(navigator.language) && !/bot|crawl|spider|slurp/i.test(navigator.userAgent) ? 'en' : lang
    try { localStorage.setItem(LANG_KEY, next) } catch { /* nothing to remember */ }
    if (next !== lang) navigate(`${pathIn(next, pathname)}${search}${next === 'en' ? '' : hash}`, { replace: true })
  // Only the first page of the visit decides.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])
  useEffect(() => { document.documentElement.lang = strings[lang].htmlLang }, [lang])
}

function Header() {
  const [open, setOpen] = useState(false)
  const { pathname } = useLocation()
  const { t, to } = useLang()
  useEffect(() => setOpen(false), [pathname])
  const nav = [
    { to: to('/'), label: t.nav.home, end: true },
    { to: to('/deploy'), label: t.nav.deploy },
    { to: to('/docs'), label: t.nav.docs },
    { to: to('/download'), label: t.nav.download },
  ]
  return <header className="site-header" data-open={open || undefined}>
    <div className="site-header-inner">
      <Link to={to('/')} className="brand" aria-label={t.nav.homeLabel}><BrandMark /><span>Open<b>VAIR</b></span></Link>
      <nav className="site-nav" aria-label={t.nav.label}>
        {nav.map(item => <NavLink key={item.to} to={item.to} end={item.end}>{item.label}</NavLink>)}
      </nav>
      <LangSwitch />
      <a className="site-github" href={GITHUB} target="_blank" rel="noreferrer"><GithubIcon /><span>GitHub</span></a>
      <button type="button" className="site-menu" aria-label={open ? t.nav.closeMenu : t.nav.openMenu} aria-expanded={open} onClick={() => setOpen(value => !value)}>{open ? <X aria-hidden="true" /> : <Menu aria-hidden="true" />}</button>
    </div>
  </header>
}

function Footer() {
  const { t: { footer }, to } = useLang()
  return <footer className="site-footer">
    <div className="site-footer-inner">
      <div className="site-footer-brand">
        <Link to={to('/')} className="brand"><BrandMark size={24} /><span>Open<b>VAIR</b></span></Link>
        <p>{footer.tagline}</p>
      </div>
      <div className="site-footer-links">
        <div><p>{footer.product}</p><Link to={to('/deploy')}>{footer.deploy}</Link><Link to={to('/download')}>{footer.desktop}</Link><Link to={to('/docs/comparison')}>{footer.comparison}</Link></div>
        <div><p>{footer.docs}</p><Link to={to('/docs/quickstart')}>{footer.quickstart}</Link><Link to={to('/docs/claude-code')}>Claude Code</Link><Link to={to('/docs/troubleshooting')}>{footer.troubleshooting}</Link></div>
        <div><p>{footer.community}</p><a href={GITHUB} target="_blank" rel="noreferrer">GitHub</a><a href={`${GITHUB}/issues`} target="_blank" rel="noreferrer">{footer.issues}</a><a href={`${GITHUB}/blob/main/SECURITY.md`} target="_blank" rel="noreferrer">{footer.security}</a></div>
      </div>
    </div>
    <p className="site-footer-legal">{footer.legal}</p>
  </footer>
}

export function Layout({ children }: { children: ReactNode }) {
  useFirstVisitLang()
  const { t } = useLang()
  return <>
    <a className="skip-link" href="#main">{t.skip}</a>
    <Header />
    <main id="main" tabIndex={-1}>{children}</main>
    <Footer />
  </>
}

export function useTitle(title?: string) {
  const { t } = useLang()
  useEffect(() => { document.title = title ? `${title} · OpenVAIR` : t.siteTitle }, [title, t])
}
