import { useState } from 'react'
import { ArrowRight, ChartColumn, Download, FlaskConical, KeyRound, Network, Radar, Users } from 'lucide-react'
import { Link } from 'react-router-dom'
import { Command } from '../components/copy'
import { BrandMark, useTitle } from '../components/layout'
import { useLang } from '../i18n'
import { INSTALL } from '../links'

/** Tools on the left, channels on the right, one OpenVAIR in between; the dashes show traffic moving. */
function RouteDiagram() {
  const { route } = useLang().t.home
  const channels = route.channels.map((name, index) => ({ name, state: index === 1 ? 'down' : 'ok' }))
  return <figure className="route" aria-label={route.label}>
    <ul className="route-side">{route.tools.map(tool => <li key={tool}>{tool}</li>)}</ul>
    <svg className="route-wires" viewBox="0 0 400 200" preserveAspectRatio="none" aria-hidden="true">
      {[25, 75, 125, 175].map(y => <path key={`in${y}`} className="route-wire" d={`M0 ${y} C 90 ${y}, 110 100, 200 100`} vectorEffect="non-scaling-stroke" />)}
      {[[40, 'ok'], [100, 'down'], [160, 'ok']].map(([y, state]) => <path key={`out${y}`} className="route-wire" data-state={state} d={`M200 100 C 290 100, 310 ${y}, 400 ${y}`} vectorEffect="non-scaling-stroke" />)}
    </svg>
    <div className="route-hub"><BrandMark size={34} /><strong>OpenVAIR</strong><small>{route.hub}</small></div>
    <ul className="route-side route-channels">{channels.map(channel => <li key={channel.name} data-state={channel.state}>{channel.name}<small>{channel.state === 'down' ? route.down : route.ok}</small></li>)}</ul>
  </figure>
}

const shots = [
  { key: 'desktop', icon: Download, src: '/docs-img/desktop-agents.webp' },
  { key: 'keys', icon: KeyRound, src: '/docs-img/keys-list.webp' },
  { key: 'channels', icon: Network, src: '/docs-img/channels-list.webp' },
  { key: 'monitor', icon: Radar, src: '/docs-img/monitor-availability.webp' },
  { key: 'usage', icon: ChartColumn, src: '/docs-img/usage-analytics.webp' },
] as const

const audienceIcons = [Users, FlaskConical]

function Gallery() {
  const { gallery } = useLang().t.home
  const [current, setCurrent] = useState<string>(shots[0].key)
  const found = shots.find(item => item.key === current) ?? shots[0]
  const shot = { ...found, ...gallery.shots[found.key] }
  return <div className="gallery">
    <div className="gallery-tabs" role="tablist" aria-label={gallery.label}>
      {shots.map(({ key, icon: Icon }) => <button key={key} type="button" role="tab" id={`tab-${key}`} aria-selected={key === current} aria-controls="gallery-panel" onClick={() => setCurrent(key)}><Icon aria-hidden="true" />{gallery.shots[key].label}</button>)}
    </div>
    <figure className="gallery-frame" id="gallery-panel" role="tabpanel" aria-labelledby={`tab-${shot.key}`}>
      <div className="window-bar" aria-hidden="true"><i /><i /><i /><span>{shot.key === 'desktop' ? 'OpenVAIR Desktop' : 'openvair.example.com'}</span></div>
      <div className="gallery-image"><img key={shot.key} src={shot.src} alt={shot.caption} /></div>
      <figcaption>{shot.caption}</figcaption>
    </figure>
  </div>
}

export function HomePage() {
  useTitle()
  const { t: { home }, to } = useLang()
  return <div className="home">
    <section className="hero">
      <div className="hero-grid" aria-hidden="true" />
      <div className="hero-copy">
        <h1><span>{home.title[0]}</span><br /><span><em>{home.titleEm}</em> {home.title[1]}</span></h1>
        <p className="hero-lead">{home.lead}</p>
        <div className="hero-actions">
          <Link to={to('/deploy')} className="button button-primary">{home.deploy}<ArrowRight aria-hidden="true" /></Link>
          <Link to={to('/download')} className="button">{home.download}</Link>
        </div>
        <Command text={INSTALL} />
      </div>
      <div className="hero-visual" aria-hidden="true">
        <div className="hero-window hero-window-back"><div className="window-bar"><i /><i /><i /><span>openvair.example.com</span></div><img src="/docs-img/keys-list.webp" alt="" /></div>
        <div className="hero-window hero-window-front"><div className="window-bar"><i /><i /><i /><span>OpenVAIR Desktop</span></div><img src="/docs-img/desktop-keys.webp" alt="" /></div>
      </div>
      <div className="hero-strip">
        <p className="kicker">{home.simple.kicker}</p>
        <ol>{home.simple.items.map(item => <li key={item.label}>
          <strong><span className="hero-strip-num">1</span>{item.unit}</strong>
          <span>{item.label}</span>
          <code>{item.chip}</code>
        </li>)}</ol>
      </div>
    </section>

    <section className="band">
      <div className="section-head">
        <p className="kicker">{home.route.kicker}</p>
        <h2>{home.route.title[0]}<br />{home.route.title[1]}</h2>
      </div>
      <RouteDiagram />
    </section>

    <section className="band">
      <div className="section-head">
        <p className="kicker">{home.audiences.kicker}</p>
        <h2>{home.audiences.title}</h2>
      </div>
      <div className="audiences">{home.audiences.items.map(({ title, anchor, points }, index) => {
        const Icon = audienceIcons[index]
        return <article key={title} className="audience">
          <span className="audience-icon"><Icon aria-hidden="true" /></span>
          <h3>{title}</h3>
          <ul>{points.map(point => <li key={point}>{point}</li>)}</ul>
          <Link to={`${to('/docs/use-cases')}#${anchor}`}>{home.audiences.more(title)}<ArrowRight aria-hidden="true" /></Link>
        </article>
      })}</div>
    </section>

    <section className="band">
      <div className="section-head">
        <p className="kicker">{home.gallery.kicker}</p>
        <h2>{home.gallery.title}</h2>
      </div>
      <Gallery />
    </section>

    <section className="cta">
      <h2>{home.cta.title}</h2>
      <p>{home.cta.body}</p>
      <Command text={INSTALL} />
      <div className="hero-actions">
        <Link to={to('/deploy')} className="button button-primary">{home.cta.guide}<ArrowRight aria-hidden="true" /></Link>
        <Link to={to('/docs/quickstart')} className="button">{home.cta.quickstart}</Link>
      </div>
    </section>
  </div>
}
