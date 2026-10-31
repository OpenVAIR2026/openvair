import { useEffect, useMemo, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { AGENT_NOTICE, HOLDER_STATUS, ipc, money, PROTOCOL_LABEL, type Agent, type Protocol, type SavedKey, type TestResult } from '../lib/ipc'
import { CheckIcon, CloseIcon, SearchIcon, WarnIcon } from './icons'

/** `connect` sets up an agent, `switch` replaces its key, `model` keeps the key. */
export type SheetMode = 'connect' | 'switch' | 'model'

/** Why a saved key cannot be written into an agent now, if it cannot. */
export function unusableReason(key: SavedKey): string | null {
  if (key.invalid) return 'Key 已失效或被撤销'
  if (key.error) return '无法连接服务器'
  if (!key.enabled) return '已被管理员停用'
  if (key.summary && !HOLDER_STATUS[key.summary.status].ok) return HOLDER_STATUS[key.summary.status].label
  return null
}

export function SetupSheet({ agent, mode, keys, keysLoading, usedBy, onClose }: { agent: Agent; mode: SheetMode; keys: SavedKey[]; keysLoading: boolean; usedBy: Record<string, string[]>; onClose: () => void }) {
  const queryClient = useQueryClient()
  const current = agent.connection
  // Changing the model keeps the protocol; connecting may pick another one.
  const [protocol, setProtocol] = useState<Protocol>(agent.protocol)
  const currentKey = current ? keys.find(key => key.id === current.key_id) : undefined
  // A key without listed models still works: the gateway lists only names it knows, so the model can be typed.
  const usable = (key: SavedKey) => !unusableReason(key)
  const listed = (key: SavedKey) => usable(key) && (key.models[protocol]?.length ?? 0) > 0
  // Prefer a key no other agent uses, so each agent can be paused on its own.
  const free = (key: SavedKey) => !usedBy[key.id]?.some(name => name !== agent.name)
  const fallback = keys.find(key => key.is_default)
  const initialKey = mode === 'model' ? current?.key_id
    : ((mode === 'connect' && fallback && listed(fallback) ? fallback : undefined) ?? keys.find(key => listed(key) && key.id !== current?.key_id && free(key)) ?? keys.find(key => listed(key) && key.id !== current?.key_id) ?? keys.find(listed) ?? keys.find(key => usable(key) && key.id !== current?.key_id) ?? keys.find(usable))?.id
  const [keyId, setKeyId] = useState(initialKey ?? '')
  const key = keys.find(item => item.id === keyId)
  const models = useMemo(() => key?.models[protocol] ?? [], [key, protocol])
  const [model, setModel] = useState(current?.model ?? '')
  const [filter, setFilter] = useState('')
  const [custom, setCustom] = useState(false)
  const [result, setResult] = useState<TestResult | null>(null)
  // Empty follows the chosen model.
  const [background, setBackground] = useState(current?.background_model ?? '')
  const manual = custom || (!!key && models.length === 0)

  // Keep the model when the new key can use it, otherwise pick the first one.
  useEffect(() => {
    if (manual || !key) return
    if (!models.includes(model)) setModel(models[0] ?? '')
  }, [key, models, model, manual])
  useEffect(() => { if (!keyId && initialKey) setKeyId(initialKey) }, [keyId, initialKey])
  // By default the list hides keys with no models for this protocol; the chosen key always stays visible.
  const [onlyListed, setOnlyListed] = useState(true)
  const hasModels = (item: SavedKey) => (item.models[protocol]?.length ?? 0) > 0
  const shownKeys = onlyListed ? keys.filter(item => hasModels(item) || item.id === keyId) : keys
  const hiddenKeys = keys.filter(item => !hasModels(item)).length
  // Switching protocol moves off a key that has nothing for it, when another one does.
  useEffect(() => {
    if (mode === 'model' || !onlyListed || !key || listed(key)) return
    const next = keys.find(item => listed(item) && free(item)) ?? keys.find(listed)
    if (next) setKeyId(next.id)
  }, [protocol])

  const preview = useQuery({
    queryKey: ['preview', agent.id, key?.id, model, models, background, protocol],
    queryFn: () => ipc.preview(agent.id, key!.id, model, models, background || null, protocol),
    enabled: !!key && !!model,
    placeholderData: previous => previous,
  })
  const asksBackground = !!preview.data?.asks_background_model
  // A background model the new key does not list follows the chosen one.
  useEffect(() => { if (background && !models.includes(background)) setBackground('') }, [background, models])
  const apply = useMutation({
    mutationFn: () => ipc.apply(agent.id, keyId, model, asksBackground && background ? background : null, protocol),
    onSuccess: value => { setResult(value); queryClient.invalidateQueries({ queryKey: ['agents'] }) },
  })
  const undo = useMutation({ mutationFn: () => ipc.undo(agent.id), onSuccess: onClose })

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape' && !apply.isPending) onClose() }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose, apply.isPending])

  const title = mode === 'connect' ? `接入 ${agent.name}` : mode === 'switch' ? '更换 Key' : '更换模型'
  const visibleModels = models.filter(name => name.toLowerCase().includes(filter.trim().toLowerCase()))
  const switching = mode === 'switch' && current && keyId !== current.key_id
  const ready = !!key && usable(key) && !!model.trim() && (mode !== 'model' || model !== current?.model) && (mode !== 'switch' || keyId !== current?.key_id || model !== current?.model || protocol !== agent.protocol)

  return <div className="scrim" onMouseDown={event => { if (event.target === event.currentTarget && !apply.isPending) onClose() }}>
    <section className="sheet" role="dialog" aria-modal="true" aria-labelledby="sheet-title">
      <div className="sheet-head">
        <div className="glyph" style={{ width: 40, height: 40, borderRadius: 11, fontSize: 14 }}>{agent.glyph}</div>
        <div style={{ flex: 1 }}>
          <h2 id="sheet-title" style={{ margin: 0, fontSize: 20, fontWeight: 600, letterSpacing: '-.02em' }}>{title}</h2>
          <div style={{ fontSize: 13, color: 'var(--muted)', marginTop: 3 }}>{agent.name} · <span className="mono">{agent.files.join('，')}</span></div>
        </div>
        <button className="btn icon-btn btn-ghost" aria-label="关闭" disabled={apply.isPending} onClick={onClose}><CloseIcon /></button>
      </div>

      {result ? <Result agent={agent} result={result} switching={!!switching} oldKey={current?.key_name} undo={() => undo.mutate()} undoing={undo.isPending} undoError={undo.error} onClose={onClose} /> : <>
        <div className="sheet-body">
          {mode !== 'connect' && current && !currentKey && !keysLoading && <div className="banner banner-warn"><WarnIcon />当前 Key「{current.key_name}」已从本机移除，请换一把 Key</div>}
          {currentKey && unusableReason(currentKey) && <div className="banner banner-warn"><WarnIcon />当前 Key「{currentKey.name}」{unusableReason(currentKey)}，请换一把 Key</div>}
          {agent.notice && <div className="banner banner-warn"><WarnIcon />{AGENT_NOTICE[agent.notice]}</div>}

          {agent.protocols.length > 1 && mode !== 'model' && <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
            <span className="label">协议</span>
            <div role="radiogroup" aria-label="协议" style={{ display: 'flex', gap: 8 }}>
              {agent.protocols.map(item => <button key={item} role="radio" aria-checked={protocol === item} className="option" style={{ flex: 1 }} onClick={() => setProtocol(item)}>
                <span className="radio" /><span style={{ fontSize: 14, fontWeight: 500 }}>{PROTOCOL_LABEL[item]}</span>
              </button>)}
            </div>
          </div>}

          {mode !== 'model' && <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
            <div style={{ display: 'flex', alignItems: 'center' }}><span className="label">KEY</span>
              {hiddenKeys > 0 && <label style={{ marginLeft: 12, display: 'flex', alignItems: 'center', gap: 6, fontSize: 12, color: 'var(--muted)', cursor: 'pointer' }}>
                <input type="checkbox" checked={onlyListed} onChange={event => setOnlyListed(event.target.checked)} style={{ accentColor: 'var(--accent)', margin: 0 }} />
                只看支持 {PROTOCOL_LABEL[protocol]} 的{onlyListed && `（隐藏 ${keys.filter(item => !hasModels(item) && item.id !== keyId).length} 把）`}
              </label>}
</div>
            {keysLoading && <div className="empty">正在读取 Key…</div>}
            {!keysLoading && keys.length === 0 && <div className="empty">还没有 Key，请先在「Key」页添加</div>}
            <div role="radiogroup" aria-label="Key" style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
              {!keysLoading && keys.length > 0 && shownKeys.length === 0 && <div className="empty">没有支持 {PROTOCOL_LABEL[protocol]} 的 Key，取消勾选可看全部</div>}
              {shownKeys.length > 0 && <div className="key-group">{shownKeys.map(item => {
                const count = item.models[protocol]?.length ?? 0
                const reason = unusableReason(item)
                return <button key={item.id} role="radio" aria-checked={keyId === item.id} className="option" disabled={!!reason} onClick={() => setKeyId(item.id)}>
                  <span className="radio" />
                  <span style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', gap: 4 }}>
                    <span style={{ fontSize: 14, fontWeight: 500 }}>{item.name}{item.id === current?.key_id && <span style={{ fontSize: 11, color: 'var(--warn)', marginLeft: 8 }}>当前</span>}{item.is_default && <span style={{ fontSize: 11, color: 'var(--accent)', marginLeft: 8 }}>默认</span>}</span>
                    <span style={{ fontSize: 12, color: 'var(--muted)' }}>{reason ?? [count ? `${count} 个模型` : '没有列出模型', `今日 ${money(item.used_cost_today_usd)}`, ...(usedBy[item.id] ?? []).filter(name => name !== agent.name).map(name => `已用于 ${name}`)].join(' · ')}</span>
                  </span>
                  <span className="chip">{item.key_prefix}…</span>
                </button>
              })}</div>}
            </div>
          </div>}

          {key && <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
            <div style={{ display: 'flex', alignItems: 'center' }}><span className="label">模型</span>
              {models.length > 0 && <button className="link-btn" style={{ marginLeft: 'auto' }} onClick={() => setCustom(value => !value)}>{custom ? '从列表选择' : '手动输入'}</button>}</div>
            {manual ? <input className="input mono" aria-label="模型名称" value={model} onChange={event => setModel(event.target.value)} placeholder="gpt-5.1-codex" spellCheck={false} autoFocus />
              : <>
                {models.length > 6 && <label className="search" style={{ maxWidth: 'none' }}><SearchIcon /><input aria-label="搜索模型" value={filter} onChange={event => setFilter(event.target.value)} placeholder="搜索模型" spellCheck={false} /></label>}
                <div className="models" role="radiogroup" aria-label="模型">
                  {visibleModels.map(name => <button key={name} role="radio" aria-checked={model === name} className="model-opt" onClick={() => setModel(name)}>
                    <span style={{ flex: 1 }}>{name}</span>{model === name && <CheckIcon size={14} />}
                  </button>)}
                  {visibleModels.length === 0 && <div className="empty" style={{ padding: 16 }}>没有匹配的模型</div>}
                </div>
                {current && model === current.model && mode !== 'connect' && <div style={{ fontSize: 12, color: 'var(--ok)', display: 'flex', alignItems: 'center', gap: 4 }}><CheckIcon size={14} />{mode === 'switch' ? '原模型在新 Key 下仍可用' : '当前模型'}</div>}
              </>}
          </div>}

          {key && asksBackground && models.length > 0 && <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
            <span className="label">后台任务模型</span>
            <select className="input mono" aria-label="后台任务模型" value={background} onChange={event => setBackground(event.target.value)}>
              <option value="">与所选模型相同{model ? `（${model}）` : ''}</option>
              {models.filter(name => name !== model).map(name => <option key={name} value={name}>{name}</option>)}
            </select>
            <div style={{ fontSize: 12, color: 'var(--soft)' }}>生成标题、压缩上下文等后台任务用这个模型，可选便宜些的</div>
          </div>}

          {preview.data && <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
            <span className="label">将要修改</span>
            <div className="diff">
              {preview.data.changes.map(change => change.before === change.after
                ? <div key={change.field} className="same">  {change.field} = {change.after}</div>
                : <div key={change.field}>
                  {change.before && <div className="del">- {change.field} = {change.before}</div>}
                  <div className="add">+ {change.field} = {change.after}</div>
                </div>)}
            </div>
            <div style={{ fontSize: 12, color: 'var(--soft)' }}>原文件自动备份，可一键还原</div>
          </div>}
          {preview.error && <div className="banner banner-error">{preview.error.message}</div>}
          {apply.error && <div className="banner banner-error" role="alert">{apply.error.message}</div>}
        </div>
        <div className="sheet-foot">
          <span style={{ flex: 1 }} />
          <button className="btn" disabled={apply.isPending} onClick={onClose}>取消</button>
          <button className="btn btn-primary" disabled={!ready || apply.isPending} onClick={() => apply.mutate()}>{apply.isPending ? '写入中…' : mode === 'connect' ? '写入并测试' : '替换并测试'}</button>
        </div>
      </>}
    </section>
  </div>
}

function Result({ agent, result, switching, oldKey, undo, undoing, undoError, onClose }: { agent: Agent; result: TestResult; switching: boolean; oldKey?: string; undo: () => void; undoing: boolean; undoError: Error | null; onClose: () => void }) {
  return <>
    <div className="sheet-body">
      {result.ok
        ? <div className="banner banner-ok" role="status"><CheckIcon />已写入，连接正常（{result.latency_ms} ms）。{agent.after_write ?? `重新打开 ${agent.name} 后生效。`}</div>
        : <div className="banner banner-error" role="alert"><WarnIcon />已写入，但测试失败：{result.message}</div>}
      {switching && oldKey && <div className="banner banner-warn"><WarnIcon /><span>旧 Key「{oldKey}」没有被停用。如果不再使用，可以在「Key」页把它从本机移除，或请管理员撤销</span></div>}
      {undoError && <div className="banner banner-error">{undoError.message}</div>}
    </div>
    <div className="sheet-foot">
      <span style={{ flex: 1 }} />
      {!result.ok && <button className="btn" disabled={undoing} onClick={undo}>{undoing ? '恢复中…' : '恢复到之前的配置'}</button>}
      <button className="btn btn-primary" onClick={onClose}>完成</button>
    </div>
  </>
}
