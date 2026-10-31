import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { HOLDER_STATUS, ipc, money, PROTOCOL_LABEL, type Protocol, type SavedKey, type Session } from '../lib/ipc'
import { AddKeyForm } from './welcome'
import { MoreIcon, RefreshIcon, WarnIcon } from '../components/icons'

/** Every key saved on this computer, one row each; a row opens to its limits and models. */
export function KeysPage({ session }: { session: Session }) {
  const queryClient = useQueryClient()
  const keys = useQuery({ queryKey: ['keys'], queryFn: ipc.keys })
  const [adding, setAdding] = useState(false)
  const [open, setOpen] = useState<string | null>(null)
  const list = keys.data ?? []

  return <main className="main">
    <div className="page-head">
      <div>
        <div className="eyebrow">本机保存</div>
        <h1 className="h1">Key</h1>
        <div className="sub">{keys.data ? <>{list.length} 把 Key · <span className="mono">{session.server_host}</span></> : '正在读取…'}</div>
      </div>
      <button className="btn" disabled={keys.isFetching} onClick={() => queryClient.invalidateQueries({ queryKey: ['keys'] })}><RefreshIcon />刷新</button>
      <button className="btn btn-primary" onClick={() => setAdding(true)}>+ 添加 Key</button>
    </div>

    {adding && <div className="card" style={{ padding: 20, display: 'flex', justifyContent: 'center' }}>
      <AddKeyForm serverHost={session.server_host} onAdded={() => setAdding(false)} onCancel={() => setAdding(false)} />
    </div>}
    {session.plain_http && <div className="banner banner-warn" style={{ display: 'flex', gap: 6, alignItems: 'center' }}><WarnIcon />{session.public_http ? '这台服务器通过公网使用 http://，Key 和请求明文传输，可能被途中的人看到和盗用' : '这台服务器使用 http://，Key 和请求在局域网中不加密传输'}</div>}
    {keys.error && <div className="banner banner-error" role="alert">{keys.error.message}</div>}
    {keys.isPending && <div className="empty">正在读取 Key…</div>}
    {list.length > 0 && <div className="card" style={{ overflow: 'visible', marginBottom: 24 }}>
      {list.map(key => <KeyRow key={key.id} item={key} open={open === key.id} onToggle={() => setOpen(current => current === key.id ? null : key.id)} />)}
    </div>}
  </main>
}

/** What the row says about the key: a tone for the dot and, when it cannot be used, why. */
function keyState(item: SavedKey): { tone: 'ok' | 'warn' | 'bad'; problem: string | null } {
  if (item.invalid) return { tone: 'bad', problem: '已失效：服务器不再认这把 Key，请向管理员要一把新的' }
  if (item.error) return { tone: 'warn', problem: `无法连接：${item.error}` }
  if (!item.summary) return { tone: 'warn', problem: null }
  const status = HOLDER_STATUS[item.summary.status]
  return status.ok ? { tone: 'ok', problem: null } : { tone: 'warn', problem: `${status.label}：${status.detail}` }
}

const TONE = { ok: 'var(--ok)', warn: 'var(--warn)', bad: 'var(--danger)' }

function KeyRow({ item, open, onToggle }: { item: SavedKey; open: boolean; onToggle: () => void }) {
  const queryClient = useQueryClient()
  const [menu, setMenu] = useState(false)
  const [menuUp, setMenuUp] = useState(false)
  const menuRef = useRef<HTMLDivElement>(null)
  const [renaming, setRenaming] = useState(false)
  const [label, setLabel] = useState(item.name)
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (!menu) return
    const close = (event: MouseEvent) => { if (!ref.current?.contains(event.target as Node)) setMenu(false) }
    document.addEventListener('mousedown', close)
    return () => document.removeEventListener('mousedown', close)
  }, [menu])
  // Open upward when the rows below leave no room, e.g. on the last key in a short window.
  useLayoutEffect(() => {
    const element = menuRef.current
    if (!menu || !element) { setMenuUp(false); return }
    const rect = element.getBoundingClientRect()
    const bottom = element.closest('.main')?.getBoundingClientRect().bottom ?? window.innerHeight
    if (rect.bottom > bottom) setMenuUp(true)
  }, [menu])
  const refresh = () => queryClient.invalidateQueries()
  const rename = useMutation({ mutationFn: () => ipc.renameKey(item.id, label.trim()), onSuccess: async () => { await refresh(); setRenaming(false) } })
  const makeDefault = useMutation({ mutationFn: () => ipc.setDefaultKey(item.id), onSuccess: refresh })
  const remove = useMutation({ mutationFn: () => ipc.removeKey(item.id), onSuccess: refresh })
  const error = rename.error ?? makeDefault.error ?? remove.error
  const { tone, problem } = keyState(item)
  const summary = item.summary
  const act = (run: () => void) => () => { setMenu(false); run() }

  return <div ref={ref} style={{ position: 'relative' }} className="key-row">
    <div className="item" style={{ cursor: renaming ? undefined : 'pointer' }} onClick={event => { if (!renaming && !(event.target as Element).closest('button, input, form, [role=menu]')) onToggle() }}>
      <span className="dot" style={{ background: TONE[tone] }} aria-hidden="true" />
      <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', gap: 3 }}>
        {renaming ? <form style={{ display: 'flex', gap: 8 }} onSubmit={event => { event.preventDefault(); rename.mutate() }}>
          <input className="input" style={{ height: 30, width: 240 }} aria-label="Key 名称" value={label} onChange={event => setLabel(event.target.value)} placeholder={item.key_name} autoFocus />
          <button className="btn" style={{ height: 30 }} type="submit" disabled={rename.isPending}>保存</button>
          <button className="btn btn-ghost" style={{ height: 30 }} type="button" onClick={() => { setRenaming(false); setLabel(item.name) }}>取消</button>
        </form> : <div className="item-k" style={{ display: 'flex', alignItems: 'center', gap: 8, minWidth: 0, whiteSpace: 'nowrap' }}>
          <span style={{ overflow: 'hidden', textOverflow: 'ellipsis' }}>{item.name}</span>
          <span className="chip">{item.key_prefix}…</span>
          {item.is_default && <span className="pill pill-ok">默认</span>}
        </div>}
        <div className="item-s" style={{ color: problem ? TONE[tone] : undefined }}>
          {problem ?? (item.used_by.length ? `${item.used_by.join('、')} 在用` : '没有 Agent 在用')}
        </div>
      </div>
      {summary && !item.invalid && <TodayUse used={summary.used_cost_today_usd} limit={summary.cost_limit_daily_usd} />}
      <div className="menu-anchor">
        <button className="btn icon-btn btn-ghost" aria-label={`${item.name} 更多操作`} aria-expanded={menu} onClick={() => setMenu(value => !value)}><MoreIcon /></button>
        {menu && <div ref={menuRef} className={menuUp ? 'menu menu-up' : 'menu'} role="menu">
          {!item.is_default && <button role="menuitem" disabled={makeDefault.isPending} onClick={act(() => makeDefault.mutate())}>设为默认</button>}
          <button role="menuitem" onClick={act(() => setRenaming(true))}>重命名</button>
          <button role="menuitem" disabled={!!item.error} onClick={act(() => ipc.openConsole('/my-key/usage'))}>在网页查看用量 ↗</button>
          <button role="menuitem" style={{ color: 'var(--danger)' }} disabled={remove.isPending} title={item.used_by.length ? '先在 Agent 页把用到它的 Agent 换成别的 Key 或断开' : '只从本机移除，不会撤销 Key'} onClick={act(() => remove.mutate())}>从本机移除</button>
        </div>}
      </div>
    </div>
    {error && <div className="item"><div className="banner banner-error" role="alert" style={{ flex: 1 }}>{error.message}</div></div>}
    {open && <KeyDetail item={item} />}
  </div>
}

/** Today's spend, with a bar when the key has a daily cap. */
function TodayUse({ used, limit }: { used: number; limit: number | null }) {
  const share = limit ? Math.min(1, used / limit) : null
  return <div style={{ width: 130, flexShrink: 0, textAlign: 'right' }}>
    <div className="mono" style={{ fontSize: 12.5 }}>今日 {money(used)}{limit !== null && <span style={{ color: 'var(--soft)' }}> / {money(limit)}</span>}</div>
    {share !== null && <div className="bar" style={{ marginTop: 4 }}><div style={{ width: `${Math.max(used > 0 ? 2 : 0, share * 100)}%`, background: share >= 1 ? 'var(--danger)' : share > .8 ? 'var(--warn)' : undefined }} /></div>}
  </div>
}

function KeyDetail({ item }: { item: SavedKey }) {
  // Model lists can be long, so they stay folded until asked for.
  const [models, setModels] = useState(false)
  const summary = item.summary
  if (!summary) return <div className="item"><div className="item-s">服务器暂时没有返回这把 Key 的额度和模型。</div></div>
  const compact = (value: number) => new Intl.NumberFormat('zh-CN', { notation: 'compact', maximumFractionDigits: 1 }).format(value)
  const limit = (used: string, cap: string | null) => cap ? `${used} / ${cap}` : `${used}（不设上限）`
  const protocols = (Object.keys(PROTOCOL_LABEL) as Protocol[]).filter(protocol => summary.models[protocol]?.length)
  const modelCount = new Set(protocols.flatMap(protocol => summary.models[protocol]!)).size
  return <div className="item" style={{ alignItems: 'flex-start', flexDirection: 'column', gap: 8, padding: '4px 18px 14px 39px', borderTop: 0 }}>
    {item.name !== item.key_name && <div className="item-s">服务器上的名称：{item.key_name}</div>}
    <div className="item-s">
      今日 Token {limit(compact(summary.used_tokens_today), summary.token_limit_daily ? compact(summary.token_limit_daily) : null)} ·
      累计费用 {limit(money(summary.used_cost_total_usd), summary.cost_limit_total_usd !== null ? money(summary.cost_limit_total_usd) : null)}
    </div>
    <div className="item-s">每分钟 {summary.requests_per_minute ? `最多 ${summary.requests_per_minute} 次请求` : '不限请求数'} · 最多 {summary.concurrency_limit} 个请求同时进行 · 今日额度在 {new Date(summary.day_ends_at).toLocaleString('zh-CN', { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' })} 重置</div>
    {protocols.length > 0 && <button className="btn btn-ghost" type="button" style={{ height: 28, padding: '0 8px', marginLeft: -8, fontSize: 12.5, color: 'var(--muted)' }} aria-expanded={models} onClick={() => setModels(value => !value)}>
      {models ? '▾' : '▸'} 可用模型 {modelCount} 个，按协议列出
    </button>}
    {models && protocols.map(protocol => <div key={protocol} style={{ display: 'flex', gap: 6, flexWrap: 'wrap', alignItems: 'center' }}>
      <span style={{ width: 90, fontSize: 12, color: 'var(--muted)' }}>{PROTOCOL_LABEL[protocol]}</span>
      {summary.models[protocol]!.map(name => <span key={name} className="chip">{name}</span>)}
    </div>)}
    {protocols.length === 0 && <div className="item-s">这把 Key 当前没有可调用的模型。如果状态正常，请联系管理员开放模型。</div>}
  </div>
}
