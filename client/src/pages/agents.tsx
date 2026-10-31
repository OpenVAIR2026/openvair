import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { AGENT_NOTICE, ipc, money, relativeTime, type Agent, type AgentNotice, type KeySummary, type ModelsUpdate, type Session } from '../lib/ipc'
import { describeAgent, type AgentState, type Tone } from '../lib/agent-state'
import { BrandMark, CheckIcon, CloseIcon, ExternalIcon, MoreIcon, WarnIcon } from '../components/icons'
import { SetupSheet, type SheetMode } from '../components/setup-sheet'
import { ManualSheet } from '../components/manual-sheet'

const LINE: Record<Tone, string> = { ok: '#78d9d0', warn: 'rgba(240,191,121,.55)', idle: 'rgba(226,246,244,.14)', off: 'rgba(226,246,244,.08)' }

export function AgentsPage({ session }: { session: Session }) {
  const { host, server_host: serverHost } = session
  const queryClient = useQueryClient()
  const agents = useQuery({ queryKey: ['agents'], queryFn: ipc.agents })
  // Dismissals stay on this computer: a notice until it changes.
  const [dismissed, dismiss] = useDismissed()
  const keys = useQuery({ queryKey: ['keys'], queryFn: ipc.keys })
  const [sheet, setSheet] = useState<{ agent: Agent; mode: SheetMode } | null>(null)
  const [manual, setManual] = useState(false)
  const refresh = () => Promise.all([queryClient.invalidateQueries({ queryKey: ['agents'] }), queryClient.invalidateQueries({ queryKey: ['keys'] })])
  const testAll = useMutation({
    mutationFn: async () => { for (const agent of agents.data ?? []) if (agent.connection) await ipc.test(agent.id).catch(() => undefined) },
    onSettled: refresh,
  })

  const [tools, setTools] = useState(false)
  const list = agents.data ?? []
  const rows = list.map(agent => ({ agent, state: describeAgent(agent, keys.data) }))
  // In use: anything set up here, or that points at OpenVAIR or needs a look. Most people use one or two.
  const inUse: Row[] = rows.filter(({ agent, state }) => agent.connection || agent.status === 'other' || state.tone === 'warn')
  const ready = rows.filter(row => !inUse.includes(row) && row.agent.installed)
  const missing = rows.filter(row => !inUse.includes(row) && !row.agent.installed)
  // Once anything is in use, the Agents on this computer go on the wheel, those in use first.
  // Ones not installed stay behind the header button.
  const wheel = [...inUse, ...ready]
  const wheelFits = inUse.length > 0 && wheel.length >= WHEEL_FROM
  const [compact, setCompact] = useCompact()
  const onWheel = wheelFits && !compact
  const attention = inUse.filter(row => row.state.tone === 'warn').length
  const usedBy: Record<string, string[]> = {}
  for (const agent of list) if (agent.connection) (usedBy[agent.connection.key_id] ??= []).push(agent.name)
  const open = (agent: Agent, mode: SheetMode) => setSheet({ agent, mode })
  const saved = keys.data ?? []
  const fallback = saved.find(key => key.is_default) ?? saved[0]
  const unreachable = saved.filter(key => key.error || key.invalid).length
  const broken = !!keys.error || (saved.length > 0 && unreachable === saved.length)
  const status = keys.error ? '无法读取 Key' : unreachable > 0 ? `${unreachable} 把 Key 异常` : '已连接'
  const hub = <Hub serverHost={serverHost} status={status} broken={broken || unreachable > 0} />

  return <main className="main">
    <div className="page-head">
      <div>
        {host && <div className="eyebrow">{host}</div>}
        <h1 className="h1">本机 Agent</h1>
        {/* With tools in use the hub shows the server, so the line only counts. */}
        <div className="sub" style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          {inUse.length > 0 ? <span>{inUse.filter(row => row.agent.connection).length} 个已接入{attention > 0 && ` · ${attention} 个需要处理`}</span> : <>
            <span className="dot" style={{ background: broken || unreachable > 0 ? 'var(--warn)' : 'var(--ok)', width: 6, height: 6 }} />
            {serverHost && <span className="mono" style={{ fontSize: 13 }}>{serverHost}</span>}
            <span>· {saved.length} 把 Key · {status}</span>
          </>}
        </div>
      </div>
      {wheelFits && <ViewSwitch compact={compact} onChange={setCompact} />}
      {inUse.some(row => row.agent.connection) && <button className="btn" disabled={testAll.isPending} onClick={() => testAll.mutate()}><CheckIcon />{testAll.isPending ? '测试中…' : '全部测试'}</button>}
      {agents.data && <button className={inUse.length > 0 ? 'btn btn-primary' : 'btn'} onClick={() => setTools(true)}>+ 接入 Agent</button>}
    </div>

    {keys.error && <div className="banner banner-warn" role="alert">无法读取 Key 列表：{keys.error.message}</div>}
    {agents.error && <div className="banner banner-error" role="alert">{agents.error.message}</div>}

    {agents.data && <>
      {/* The compact list takes the wheel's place, centred where the wheel would turn. */}
      {/* A few tools sit in the middle of the page rather than under the header. */}
      {inUse.length > 0 && <div style={wheelFits ? { flex: '1 1 0', minHeight: 'min-content', maxHeight: DRUM.height, display: 'flex', flexDirection: 'column', justifyContent: 'center' } : { flex: '1 1 0', minHeight: 'min-content', display: 'flex', flexDirection: 'column', justifyContent: 'center', gap: 28, paddingBottom: 24 }}>
        {onWheel
          ? <Wheel hub={hub} rows={wheel} split={inUse.length} card={({ agent, state }, front) => <AgentCard agent={agent} state={state} notice={front && agent.notice && !dismissed.includes(`notice:${agent.notice}`) ? agent.notice : null} onDismiss={() => dismiss(`notice:${agent.notice}`)} onOpen={mode => open(agent, mode)} onChanged={refresh} />} />
          : <Map hub={hub} dense={wheelFits} tones={inUse.map(row => row.state.tone)}>
            {inUse.map(({ agent, state }) => <AgentCard key={agent.id} dense={wheelFits} agent={agent} state={state} notice={agent.notice && !dismissed.includes(`notice:${agent.notice}`) ? agent.notice : null} onDismiss={() => dismiss(`notice:${agent.notice}`)} onOpen={mode => open(agent, mode)} onChanged={refresh} />)}
          </Map>}
        {!onWheel && <Shelf ready={ready} onConnect={agent => open(agent, 'connect')} onMore={() => setTools(true)} />}
      </div>}

      {inUse.length === 0 && ready.length === 1 ? <div className="start">
        <div className="glyph" style={{ width: 56, height: 56, fontSize: 18 }}>{ready[0].agent.glyph}</div>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontSize: 18, fontWeight: 600 }}>用 OpenVAIR 接入 {ready[0].agent.name}</div>
          <div className="sub" style={{ fontSize: 13 }}>本机检测到 {ready[0].agent.name}。接入后它会通过你的 OpenVAIR Key 访问模型，修改前会自动备份配置。</div>
          {ready[0].agent.notice && <div style={{ marginTop: 8, fontSize: 12, color: 'var(--warn)' }}>{AGENT_NOTICE[ready[0].agent.notice]}</div>}
        </div>
        <button className="btn btn-primary" onClick={() => open(ready[0].agent, 'connect')}>开始接入</button>
      </div> : inUse.length === 0 && ready.length > 0 && <>
        <div className="section-label">选一个 Agent 开始</div>
        <div className="agent-list">
          {ready.map(({ agent, state }) => <AgentRow key={agent.id} agent={agent} state={state} onConnect={() => open(agent, 'connect')} />)}
        </div>
      </>}
      {inUse.length === 0 && ready.length === 0 && <div className="start">
        <div style={{ flex: 1 }}>
          <div style={{ fontSize: 18, fontWeight: 600 }}>本机还没有检测到支持的 Agent</div>
          <div className="sub" style={{ fontSize: 13 }}>安装一个支持的 Agent 后回到这里接入，或者手动把 OpenVAIR 填进其他客户端。</div>
        </div>
        <button className="btn btn-primary" onClick={() => setTools(true)}>查看可接入的 Agent</button>
      </div>}
    </>}

    <footer className="statusbar">
      <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}><span className="dot" style={{ background: broken || unreachable > 0 ? 'var(--warn)' : 'var(--ok)', width: 6, height: 6 }} />{fallback ? `默认 Key · ${fallback.name}` : '还没有 Key'}</span>
      <span>本机保存 {saved.length} 把 Key</span>
      <span>修改前自动备份配置</span>
      {fallback && <a style={{ marginLeft: 'auto', fontFamily: 'var(--font)', fontSize: 12 }} onClick={() => ipc.openConsole('/my-key/usage')}>查看用量 ↗</a>}
    </footer>

    {tools && <ToolsSheet ready={ready} missing={missing} onConnect={agent => { setTools(false); open(agent, 'connect') }} onManual={() => { setTools(false); setManual(true) }} onClose={() => setTools(false)} />}
    {manual && <ManualSheet keys={saved} keysLoading={keys.isPending} onClose={() => setManual(false)} />}
    {sheet && <SetupSheet agent={sheet.agent} mode={sheet.mode} keys={saved} keysLoading={keys.isPending} usedBy={usedBy} onClose={() => { setSheet(null); refresh() }} />}
  </main>
}

/** Turns the compact list on in place of the wheel. */
function ViewSwitch({ compact, onChange }: { compact: boolean; onChange: (compact: boolean) => void }) {
  return <label className="view-switch" title="只列出正在使用的 Agent，不用滚筒">
    精简
    <button className="switch switch-sm" role="switch" aria-label="精简模式" aria-checked={compact} onClick={() => onChange(!compact)}><span /></button>
  </label>
}

const DISMISSED = 'openvair.dismissed-banners'
const COMPACT = 'openvair.compact-agents'

/** Whether many Agents show as the plain list instead of the wheel, kept on this computer. */
function useCompact(): [boolean, (compact: boolean) => void] {
  const [compact, setCompact] = useState(() => localStorage.getItem(COMPACT) === '1')
  const set = (next: boolean) => {
    localStorage.setItem(COMPACT, next ? '1' : '0')
    setCompact(next)
  }
  return [compact, set]
}

function useDismissed(): [string[], (id: string) => void] {
  const [ids, setIds] = useState<string[]>(() => {
    try { return JSON.parse(localStorage.getItem(DISMISSED) ?? '[]') } catch { return [] }
  })
  const dismiss = (id: string) => {
    const next = [...ids.filter(item => item !== id), id]
    localStorage.setItem(DISMISSED, JSON.stringify(next))
    setIds(next)
  }
  return [ids, dismiss]
}

/** The latest background update of the model lists, for a day. */
function ModelsNote({ update }: { update: ModelsUpdate | null }) {
  if (!update || Date.now() - new Date(update.at).getTime() > 24 * 3600 * 1000) return null
  const time = new Date(update.at).toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })
  const parts = [update.added.length && `新增 ${update.added.join(', ')}`, update.removed.length && `移除 ${update.removed.join(', ')}`].filter(Boolean)
  return <span style={{ color: 'var(--muted)' }} title={`模型列表已于 ${time} 更新：${parts.join('；')}`}>模型列表 {time} 已更新</span>
}

/** The server on the left, linked to each tool in use. Cards vary in height, so the links follow their measured centres. */
function Map({ hub, dense, tones, children }: { hub: React.ReactNode; dense?: boolean; tones: Tone[]; children: React.ReactNode }) {
  const ref = useRef<HTMLDivElement>(null)
  const [layout, setLayout] = useState<{ height: number; centers: number[] }>({ height: 0, centers: [] })
  useLayoutEffect(() => {
    const list = ref.current
    if (!list) return
    const measure = () => setLayout({ height: list.offsetHeight, centers: Array.from(list.children as HTMLCollectionOf<HTMLElement>, card => card.offsetTop - list.offsetTop + card.offsetHeight / 2) })
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(list)
    for (const card of Array.from(list.children)) observer.observe(card)
    return () => observer.disconnect()
  }, [tones.length])
  const { height, centers } = layout
  return <div className="map">
    {hub}
    <svg height={height} viewBox={`0 0 100 ${height || 1}`} preserveAspectRatio="none" aria-hidden="true">
      {centers.map((y, index) => {
        const d = `M0 ${height / 2} C 50 ${height / 2}, 50 ${y}, 100 ${y}`
        return tones[index] === 'ok'
          ? <g key={index}><path d={d} stroke="rgba(120,217,208,.18)" strokeWidth="2" fill="none" vectorEffect="non-scaling-stroke" /><path className="flow" d={d} stroke={LINE.ok} strokeWidth="2" fill="none" vectorEffect="non-scaling-stroke" /></g>
          : <path key={index} d={d} stroke={LINE[tones[index]]} strokeWidth="2" strokeDasharray="2 6" fill="none" vectorEffect="non-scaling-stroke" />
      })}
    </svg>
    <div className={dense ? 'agents agents-dense' : 'agents'} ref={ref}>{children}</div>
  </div>
}

/** The server every agent here goes through. */
function Hub({ serverHost, status, broken }: { serverHost: string | null; status: string; broken: boolean }) {
  return <div className="hub">
    <div className="hub-mark"><BrandMark size={44} /></div>
    <div style={{ textAlign: 'center' }}>
      <div style={{ fontSize: 15, fontWeight: 600, letterSpacing: '.06em' }}>OpenVAIR</div>
      <div className="mono" style={{ fontSize: 11.5, color: 'var(--muted)', marginTop: 4 }}>{serverHost}</div>
    </div>
    <div style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 12, color: broken ? 'var(--warn)' : 'var(--ok)' }}><span className="dot pulse" style={{ background: 'currentColor' }} />{status}</div>
  </div>
}

const WHEEL_FROM = 4
const DRUM = { height: 460, least: 300, card: 104, step: 25, perspective: 1000 }
// Radius that keeps neighbouring cards a card plus a gap apart at the front.
const RADIUS = (DRUM.card + 14) / (2 * Math.sin(DRUM.step * Math.PI / 360))
const DOT: Record<Tone, string> = { ok: '#78d9d0', warn: '#f0bf79', idle: '#5d6f73', off: '#3a4a50' }
const clamp = (value: number, low: number, high: number) => Math.min(high, Math.max(low, value))
// The drum moves like an iOS picker: it follows the finger one to one, keeps
// a flick's speed and slows down onto a slot, and springs back from an end.
const SLOT = DRUM.card + 14
// How far a flick carries, in ms of its release speed; also the glide's time constant.
const GLIDE = 325
const NEAR = .5 / SLOT
// Stiffness of the critically damped spring that settles taps, notches and ends, per ms.
const SPRING = .016
// Past either end the drum still gives, less the further it is pulled, never more than half a slot.
const rubber = (value: number, count: number) => {
  const end = count - 1
  const give = (over: number) => .5 * over / (over + .5)
  return value < 0 ? -give(-value) : value > end ? end + give(value - end) : value
}

/**
 * Once a tool is in use the Agents on this computer sit on a drum, like a picker wheel: the one in front is a
 * full card linked to the server, the others tilt away but keep their name and
 * status, and the rail beside it shows every tool's state at once. It opens
 * on the first tool that needs a look.
 */
function Wheel({ hub, rows, split, card }: { hub: React.ReactNode; rows: Row[]; split: number; card: (row: Row, front: boolean) => React.ReactNode }) {
  const count = rows.length
  const first = Math.max(0, rows.findIndex(row => row.state.tone === 'warn'))
  const still = useRef(matchMedia('(prefers-reduced-motion: reduce)').matches)
  // `target` is the slot the drum rests on or is heading for; `posRef` is where it is drawn.
  // Turning paints the cards straight onto the DOM and renders nothing: `front`,
  // the card that is lit and unfolded, only changes once the drum rests on it.
  const target = useRef(first)
  const posRef = useRef(still.current ? first : first + 2.5)
  const [front, setFront] = useState(first)
  // The front card unfolds its notice only once the drum stands still, so
  // passing cards never change height, and push the others, mid-turn.
  const [resting, setResting] = useState(still.current)
  const paint = useRef((_: number) => {})
  const frame = useRef(0)
  // Recent positions, so a release or a new notch carries on at the speed the drum already has.
  const trail = useRef<{ t: number; p: number }[]>([])
  const draw = (p: number) => {
    const t = performance.now()
    posRef.current = p
    paint.current(p)
    setResting(false)
    trail.current = [...trail.current.filter(point => t - point.t < 100), { t, p }]
  }
  // Speed in slots per ms over the last 100ms before `at`.
  const speed = (at = performance.now()) => {
    const recent = trail.current.filter(point => at - point.t < 100)
    if (recent.length < 2) return 0
    const a = recent[0], b = recent[recent.length - 1]
    return b.t > a.t ? (b.p - a.p) / (b.t - a.t) : 0
  }
  const hold = (p: number) => { cancelAnimationFrame(frame.current); draw(p) }
  // Comes to rest on a slot. Moving towards it fast enough, the drum glides on
  // and slows down exactly there; otherwise a spring brings it in.
  const settle = (index: number, velocity = speed()) => {
    cancelAnimationFrame(frame.current)
    const to = clamp(Math.round(index), 0, count - 1)
    target.current = to
    const from = posRef.current
    const distance = to - from
    const rest = () => { draw(to); setFront(to); setResting(true) }
    if (still.current) { rest(); return }
    const tau = distance / velocity
    const glide = velocity * distance > 0 && tau > 80 && tau < 500
    const a = from - to, b = velocity + SPRING * a
    const at = (t: number) => glide ? from + distance * (1 - Math.exp(-t / tau)) : to + (a + b * t) * Math.exp(-SPRING * t)
    const begin = performance.now()
    const tick = (now: number) => {
      const t = Math.max(0, now - begin)
      const p = at(t)
      // Done once it is there and stays there, not when a spring merely passes through.
      // Half a pixel off is there: an exponential glide would otherwise crawl on for seconds.
      if (Math.abs(p - to) < NEAR && Math.abs(at(t + 16) - to) < NEAR) { rest(); return }
      draw(p)
      frame.current = requestAnimationFrame(tick)
    }
    frame.current = requestAnimationFrame(tick)
  }
  // Lets go at `velocity`: the flick picks the slot it would coast to.
  const fling = (velocity: number) => settle(posRef.current + velocity * GLIDE, velocity)
  const go = (index: number) => settle(index)
  useEffect(() => { settle(target.current, 0); return () => cancelAnimationFrame(frame.current) }, [count])

  const drum = useRef<HTMLDivElement>(null)
  // The drum takes the room the page leaves, so the status bar below stays on screen.
  const box = useRef<HTMLDivElement>(null)
  const [height, setHeight] = useState(DRUM.height)
  useLayoutEffect(() => {
    const element = box.current
    if (!element) return
    const measure = () => setHeight(clamp(element.clientHeight, DRUM.least, DRUM.height))
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(element)
    return () => observer.disconnect()
  }, [])

  // Wheel and trackpad turn the drum only while it is fully on screen. Each
  // gesture, momentum included, either turns the drum or scrolls the page,
  // never both: at either end a new gesture scrolls the page. A trackpad moves
  // the drum one to one (macOS adds the momentum) and it snaps once the events
  // stop; a mouse wheel turns it a slot per notch.
  useEffect(() => {
    const element = drum.current
    if (!element) return
    let release = 0
    let last = -Infinity
    let turning = false
    // Direction the gesture ran into an end in: the drum bounced off it and the
    // rest of the push that way is dropped, but turning back works at once.
    let spent = 0
    let raw = 0
    // Whether the drum is on screen when a gesture begins. Checked once: the
    // page's elastic overscroll shifts it mid-gesture.
    let seen = false
    const page = () => element.closest('.main') ?? document.documentElement
    const onScreen = () => {
      const box = element.getBoundingClientRect()
      const view = page().getBoundingClientRect()
      return box.top >= view.top - 1 && box.bottom <= view.bottom + 1
    }
    // Whether the page itself can scroll further that way.
    const pageMoves = (down: boolean) => {
      const view = page()
      return down ? view.scrollTop + view.clientHeight < view.scrollHeight - 1 : view.scrollTop > 0
    }
    const onWheel = (event: WheelEvent) => {
      const at = posRef.current
      const atEnd = (event.deltaY > 0 && at >= count - 1.01) || (event.deltaY < 0 && at <= .01)
      const vertical = Math.abs(event.deltaY) >= Math.abs(event.deltaX)
      if (event.timeStamp - last > 200) {
        seen = onScreen()
        // At an end the page takes the gesture, unless it has nowhere to go
        // either: then the drum keeps it rather than let the page bounce.
        turning = vertical && seen && (!atEnd || !pageMoves(event.deltaY > 0))
        raw = at
        spent = atEnd ? Math.sign(event.deltaY) : 0
      } else if (!turning && !atEnd && vertical && event.deltaY !== 0 && seen) {
        // A gesture that began by pushing past an end can still turn back into the drum.
        turning = true
        raw = at
      }
      last = event.timeStamp
      if (!turning) return
      event.preventDefault()
      // WebKit and Chrome report a trackpad's legacy wheelDeltaY as -3 × deltaY; a mouse notch breaks that.
      const legacy = (event as WheelEvent & { wheelDeltaY?: number }).wheelDeltaY
      const notch = event.deltaMode !== 0 || (!!legacy && Math.abs(legacy + 3 * event.deltaY) > 1)
      if (notch) {
        settle(target.current + Math.sign(event.deltaY))
        raw = target.current
        return
      }
      if (spent && Math.sign(event.deltaY) === spent) return
      if (spent) { spent = 0; raw = at }
      const before = raw
      raw += event.deltaY / SLOT
      const end = count - 1
      const velocity = speed()
      // Carried into an end at speed (a flick or macOS momentum), it bounces
      // off like iOS instead of waiting, stretched, for the momentum to run out.
      if ((raw > end || raw < 0) && before >= 0 && before <= end && Math.abs(velocity) > .002) {
        spent = raw > end ? 1 : -1
        clearTimeout(release)
        settle(raw > end ? end : 0, clamp(velocity, -.02, .02))
        return
      }
      hold(rubber(raw, count))
      clearTimeout(release)
      const stamp = performance.now()
      release = window.setTimeout(() => fling(speed(stamp)), 90)
    }
    element.addEventListener('wheel', onWheel, { passive: false })
    return () => { element.removeEventListener('wheel', onWheel); clearTimeout(release) }
  }, [count])

  // Dragging the drum turns it too; a drag never counts as a click on a card.
  const drag = useRef<{ y: number; from: number; moved: boolean } | null>(null)
  const [dragging, setDragging] = useState(false)
  const onPointerDown = (event: React.PointerEvent) => {
    if (event.button !== 0 || (event.target as Element).closest('button, a, input, [role=menu]')) return
    // Touching a turning drum stops it where it is, as on iOS.
    cancelAnimationFrame(frame.current)
    drag.current = { y: event.clientY, from: posRef.current, moved: false }
  }
  const onPointerMove = (event: React.PointerEvent) => {
    const state = drag.current
    if (!state) return
    const dy = event.clientY - state.y
    if (!state.moved && Math.abs(dy) < 4) return
    if (!state.moved) { state.moved = true; setDragging(true); drum.current?.setPointerCapture(event.pointerId) }
    hold(rubber(state.from - dy / SLOT, count))
  }
  const onPointerUp = () => {
    if (drag.current?.moved) fling(speed())
    else if (drag.current) settle(posRef.current, 0)
    setTimeout(() => { drag.current = null }, 0)
    setDragging(false)
  }

  // Cards only re-render when the drum starts turning or comes to rest.
  const cards = useMemo(() => rows.map((row, index) => card(row, index === front && resting)), [rows, card, front, resting])
  // The front card grows with a notice; the others make room for it.
  const frontRef = useRef<HTMLDivElement>(null)
  const [frontHeight, setFrontHeight] = useState(DRUM.card)
  useLayoutEffect(() => {
    const element = frontRef.current
    if (!element) return
    const measure = () => setFrontHeight(element.offsetHeight)
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(element)
    return () => observer.disconnect()
  }, [front])
  // Room the front card needs beyond a plain one; neighbours ease aside when it grows.
  const room = useRef(0)
  useEffect(() => {
    const to = Math.max(0, frontHeight - DRUM.card) / 2
    const from = room.current
    if (from === to) return
    if (still.current) { room.current = to; paint.current(posRef.current); return }
    const begin = performance.now()
    let id = 0
    const tick = (now: number) => {
      const t = Math.min(1, (now - begin) / 220)
      room.current = from + (to - from) * (1 - (1 - t) ** 3)
      paint.current(posRef.current)
      if (t < 1) id = requestAnimationFrame(tick)
    }
    id = requestAnimationFrame(tick)
    return () => cancelAnimationFrame(id)
  }, [frontHeight])

  const place = (index: number, at = posRef.current) => {
    const offset = index - at
    const angle = clamp(offset * DRUM.step, -90, 90)
    const radians = angle * Math.PI / 180
    const y = RADIUS * Math.sin(radians) + room.current * clamp(offset, -1, 1)
    const z = RADIUS * (Math.cos(radians) - 1)
    // Where the card's centre lands on screen once perspective shrinks it.
    return { offset, angle, y, z, seen: y * DRUM.perspective / (DRUM.perspective - z) }
  }
  const half = height / 2

  // Only transform and opacity change while turning, so the compositor can move the cards without repainting them.
  const slotLook = (index: number, at?: number) => {
    const { offset, angle, y, z } = place(index, at)
    const distance = Math.abs(offset)
    return {
      visibility: distance > 3.4 ? 'hidden' as const : 'visible' as const,
      transform: `translate3d(0, calc(-50% + ${y}px), ${z}px) rotateX(${-angle}deg)`,
      opacity: clamp(1.1 - distance * .38, 0, 1),
      zIndex: 20 - Math.round(distance * 4),
    }
  }
  const linkLook = (index: number, at?: number) => {
    const { offset, seen } = place(index, at)
    const distance = Math.abs(offset)
    const y = half + seen
    const near = clamp(1 - distance, 0, 1)
    return {
      d: `M0 ${half} C 50 ${half}, 50 ${y}, 100 ${y}`,
      g: { visibility: distance > 2.6 ? 'hidden' as const : 'visible' as const, opacity: clamp(.55 - distance * .18, 0, 1) + near * .45 },
      flow: { opacity: distance < .5 ? 1 : 0 },
    }
  }
  const slots = useRef<(HTMLDivElement | null)[]>([])
  const links = useRef<(SVGGElement | null)[]>([])
  paint.current = at => {
    rows.forEach((_, index) => {
      const slot = slots.current[index]
      if (slot) Object.assign(slot.style, slotLook(index, at))
      const link = links.current[index]
      if (!link) return
      const look = linkLook(index, at)
      Object.assign(link.style, look.g)
      link.querySelectorAll('path').forEach(path => {
        path.setAttribute('d', look.d)
        if (path.classList.contains('flow')) path.style.opacity = String(look.flow.opacity)
      })
    })
  }

  return <div ref={box} className="map" style={{ flex: '1 1 0', minHeight: DRUM.least, maxHeight: DRUM.height }}>
    {hub}
    <svg height={height} viewBox={`0 0 100 ${height}`} preserveAspectRatio="none" aria-hidden="true">
      {rows.map(({ state }, index) => {
        if (index >= split) return null
        const look = linkLook(index)
        return <g key={index} ref={element => { links.current[index] = element }} style={look.g}>
          {state.tone === 'ok'
            ? <><path d={look.d} stroke="rgba(120,217,208,.18)" strokeWidth="2" fill="none" vectorEffect="non-scaling-stroke" /><path className="flow" d={look.d} style={look.flow} stroke={LINE.ok} strokeWidth="2.5" fill="none" vectorEffect="non-scaling-stroke" /></>
            : <path d={look.d} stroke={LINE[state.tone]} strokeWidth="2" strokeDasharray="2 6" fill="none" vectorEffect="non-scaling-stroke" />}
        </g>
      })}
    </svg>
    <div className="wheel">
      <div ref={drum} className={dragging ? 'drum dragging' : 'drum'} style={{ height }} tabIndex={0} role="listbox" aria-label="正在使用的 Agent" aria-activedescendant={`drum-${rows[front].agent.id}`}
        onKeyDown={event => {
          const to = { ArrowUp: target.current - 1, ArrowDown: target.current + 1, Home: 0, End: count - 1 }[event.key]
          if (to === undefined || (event.target as Element) !== drum.current) return
          event.preventDefault()
          go(to)
        }}
        onPointerDown={onPointerDown} onPointerMove={onPointerMove} onPointerUp={onPointerUp} onPointerCancel={onPointerUp}>
        <div className={`drum-lens drum-lens-${rows[front].state.tone}`} style={{ height: frontHeight + 18 }} />
        {rows.map((row, index) => {
          const isFront = index === front
          return <div key={row.agent.id} id={`drum-${row.agent.id}`} ref={element => { slots.current[index] = element; if (isFront) frontRef.current = element }} role="option" aria-selected={isFront} inert={!isFront}
            className={isFront && resting ? 'drum-slot front' : 'drum-slot'}
            style={slotLook(index)}
            onClickCapture={event => {
              if (drag.current?.moved || !isFront) { event.preventDefault(); event.stopPropagation() }
              if (!drag.current?.moved && !isFront) go(index)
            }}>
            {cards[index]}
          </div>
        })}
      </div>
      <div className="drum-rail" aria-hidden="true">
        {rows.map(({ agent, state }, index) => <button key={agent.id} tabIndex={-1} title={agent.name} aria-current={index === front} className={index === split ? 'drum-rail-split' : undefined} style={{ '--tone': DOT[state.tone] } as React.CSSProperties} onClick={() => go(index)}><span /></button>)}
        <div className="drum-rail-count mono">{front + 1}/{count}</div>
      </div>
    </div>
  </div>
}

function AgentCard({ agent, state, notice, dense, onDismiss, onOpen, onChanged }: { dense?: boolean; agent: Agent; state: AgentState; notice: AgentNotice | null; onDismiss: () => void; onOpen: (mode: SheetMode) => void; onChanged: () => void }) {
  const [menu, setMenu] = useState(false)
  const menuRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (!menu) return
    const close = (event: MouseEvent) => { if (!menuRef.current?.contains(event.target as Node)) setMenu(false) }
    window.addEventListener('mousedown', close)
    return () => window.removeEventListener('mousedown', close)
  }, [menu])
  const test = useMutation({ mutationFn: () => ipc.test(agent.id), onSettled: onChanged })
  const disconnect = useMutation({ mutationFn: () => ipc.disconnect(agent.id), onSettled: onChanged })
  const connection = agent.connection
  const pill = state.tone === 'ok' ? 'pill-ok' : state.tone === 'warn' ? 'pill-warn' : 'pill-idle'
  const glyphColor = state.tone === 'ok' ? 'var(--accent-hi)' : state.tone === 'warn' ? 'var(--warn)' : 'var(--faint)'

  return <div className={`agent agent-${state.tone}`} ref={menuRef} style={{ height: 'auto', minHeight: dense ? 0 : 88, flexWrap: 'wrap' }}>
    <span className="agent-dot" style={{ background: state.tone === 'ok' ? '#78d9d0' : state.tone === 'warn' ? '#f0bf79' : '#3a4a50' }} />
    <div className="glyph" style={{ color: glyphColor }}>{agent.glyph}</div>
    <div className="agent-body">
      <div className="agent-title">{agent.name}<span className={`pill ${pill}`} title={state.label} style={{ maxWidth: 260 }}>{state.label}</span></div>
      <div className="agent-meta">
        {connection ? <><span className="shrink" title={connection.key_name}>{connection.key_name}</span><span className="chip key-prefix">{connection.key_prefix}…</span><span className="chip" title={connection.model}>{connection.model}</span></>
          : <span title={agent.files[0]}>{agent.files[0]}</span>}
        {connection?.background_missing && <span style={{ color: 'var(--warn)' }} title={`Key 的模型列表里已没有 ${connection.background_model}，Claude Code 的后台任务会失败，请更换模型重新写入`}>后台任务模型已不可用</span>}
        {connection && <ModelsNote update={connection.models_update} />}
        {test.error && <span style={{ color: 'var(--danger)' }} title={test.error.message}>{test.error.message}</span>}
        {disconnect.error && <span style={{ color: 'var(--danger)' }} title={disconnect.error.message}>{disconnect.error.message}</span>}
      </div>
      {connection && state.key?.summary && !state.key.invalid && <KeyToday summary={state.key.summary} />}
    </div>
    <div className="agent-actions">
      {state.action === 'install' && <a className="btn btn-ghost" onClick={() => ipc.openExternal(agent.homepage)}>安装说明 <ExternalIcon /></a>}
      {state.action === 'connect' && <button className="btn btn-primary" onClick={() => onOpen('connect')}>接入</button>}
      {connection && state.tone === 'ok' && <button className="btn" disabled={test.isPending} onClick={() => test.mutate()}>{test.isPending ? '测试中…' : '测试'}</button>}
      {state.action === 'switch' && <button className={state.tone === 'warn' ? 'btn btn-primary' : 'btn'} onClick={() => onOpen('switch')}>更换 Key</button>}
      {connection && <button className="btn icon-btn btn-ghost" aria-label={`${agent.name} 更多操作`} aria-expanded={menu} onClick={() => setMenu(open => !open)}><MoreIcon /></button>}
    </div>
    {menu && connection && <div className="menu" role="menu">
      <button role="menuitem" onClick={() => { setMenu(false); onOpen('model') }}>更换模型</button>
      <button role="menuitem" onClick={() => { setMenu(false); test.mutate() }}>测试连接</button>
      <button role="menuitem" style={{ color: 'var(--danger)' }} onClick={() => { setMenu(false); disconnect.mutate() }}>断开并还原配置</button>
    </div>}
    {notice && <div className="agent-notice"><WarnIcon /><span style={{ flex: 1 }}>{AGENT_NOTICE[notice]}</span><button className="btn icon-btn btn-ghost" aria-label="不再提示" onClick={onDismiss}><CloseIcon /></button></div>}
  </div>
}

type Row = { agent: Agent; state: AgentState }

/** Today's spend on the key the agent uses, and when that key last carried a request. Keys are shared, so this is per key, not per agent. */
function KeyToday({ summary }: { summary: KeySummary }) {
  const limit = summary.cost_limit_daily_usd
  const share = limit ? Math.min(1, summary.used_cost_today_usd / limit) : null
  return <div className="agent-usage" title="这把 Key 今天的费用；多个 Agent 共用一把 Key 时合在一起算">
    {share !== null && <div className="bar"><div style={{ width: `${Math.max(summary.used_cost_today_usd > 0 ? 2 : 0, share * 100)}%`, background: share >= 1 ? 'var(--danger)' : share > .8 ? 'var(--warn)' : undefined }} /></div>}
    <span className="mono" style={{ color: 'var(--ink-2)' }}>Key 今日 {money(summary.used_cost_today_usd)}{limit !== null && <span style={{ color: 'var(--soft)' }}> / {money(limit)}</span>}</span>
    <span>{summary.last_used_at ? `最近请求 ${relativeTime(summary.last_used_at)}` : '今天还没有请求'}</span>
  </div>
}

/** Tools installed here but not set up, as tiles under the map, plus a way to the rest. */
function Shelf({ ready, onConnect, onMore }: { ready: Row[]; onConnect: (agent: Agent) => void; onMore: () => void }) {
  return <div className="shelf">
    <div className="section-label" style={{ margin: 0 }}>{ready.length > 0 ? '本机还可接入' : '接入更多'}</div>
    <div className="shelf-tiles">
      {ready.map(({ agent }) => <button key={agent.id} className="shelf-tile" onClick={() => onConnect(agent)} title={agent.notice ? AGENT_NOTICE[agent.notice] : agent.files[0]}>
        <span className="glyph glyph-sm">{agent.glyph}</span>
        <span style={{ flex: 1, minWidth: 0, textAlign: 'left' }}><span className="shelf-name">{agent.name}</span><span className="shelf-sub">已安装 · 一键接入</span></span>
        <span className="shelf-go">接入</span>
      </button>)}
      <button className="shelf-tile shelf-more" onClick={onMore}>
        <span className="glyph glyph-sm" style={{ color: 'var(--faint)' }}>+</span>
        <span style={{ flex: 1, minWidth: 0, textAlign: 'left' }}><span className="shelf-name">更多 Agent</span><span className="shelf-sub">未安装的、手动接入其他客户端</span></span>
      </button>
    </div>
  </div>
}

/** Every tool not in use yet, behind the header button: installed ones first, then the rest and manual setup. */
function ToolsSheet({ ready, missing, onConnect, onManual, onClose }: { ready: Row[]; missing: Row[]; onConnect: (agent: Agent) => void; onManual: () => void; onClose: () => void }) {
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape') onClose() }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])
  const group = (label: string, rows: Row[]) => rows.length > 0 && <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
    <span className="label">{label}</span>
    <div className="agent-list">{rows.map(({ agent, state }) => <AgentRow key={agent.id} agent={agent} state={state} onConnect={() => onConnect(agent)} />)}</div>
  </div>

  return <div className="scrim" onMouseDown={event => { if (event.target === event.currentTarget) onClose() }}>
    <section className="sheet" role="dialog" aria-modal="true" aria-labelledby="tools-title">
      <div className="sheet-head">
        <div className="glyph" style={{ width: 40, height: 40, borderRadius: 11, fontSize: 14 }}>+</div>
        <div style={{ flex: 1 }}>
          <h2 id="tools-title" style={{ margin: 0, fontSize: 20, fontWeight: 600, letterSpacing: '-.02em' }}>接入 Agent</h2>
          <div style={{ fontSize: 13, color: 'var(--muted)', marginTop: 3 }}>OpenVAIR Desktop 会改写 Agent 的配置文件，修改前自动备份，断开时还原</div>
        </div>
        <button className="btn icon-btn btn-ghost" aria-label="关闭" onClick={onClose}><CloseIcon /></button>
      </div>
      <div className="sheet-body">
        {group('本机已安装', ready)}
        {group('没有检测到', missing)}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
          <span className="label">其他客户端</span>
          <div className="agent-list"><div className="agent-row">
            <div className="glyph glyph-sm" style={{ color: 'var(--faint)' }}>+</div>
            <div className="agent-body" style={{ gap: 2 }}>
              <div className="agent-row-title">手动接入</div>
              <div className="agent-meta"><span>Cursor、Cherry Studio、Cline 等，复制地址和 Key 自己填</span></div>
            </div>
            <button className="btn" onClick={onManual}>查看接入信息</button>
          </div></div>
        </div>
      </div>
    </section>
  </div>
}

/** A tool that is not in use: one line with its main action. */
function AgentRow({ agent, state, onConnect }: { agent: Agent; state: AgentState; onConnect: () => void }) {
  return <div className="agent-row">
    <div className="glyph glyph-sm" style={{ color: agent.installed ? 'var(--accent-hi)' : 'var(--faint)' }}>{agent.glyph}</div>
    <div className="agent-body" style={{ gap: 2 }}>
      <div className="agent-row-title">{agent.name}</div>
      <div className="agent-meta">
        {agent.notice ? <span style={{ color: 'var(--warn)' }} title={AGENT_NOTICE[agent.notice]}>{AGENT_NOTICE[agent.notice]}</span> : <span title={agent.files[0]}>{agent.installed ? agent.files[0] : state.label}</span>}
      </div>
    </div>
    {state.action === 'install'
      ? <a className="btn btn-ghost" onClick={() => ipc.openExternal(agent.homepage)}>安装说明 <ExternalIcon /></a>
      : <button className="btn" onClick={onConnect}>接入</button>}
  </div>
}
