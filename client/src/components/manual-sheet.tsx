import { useEffect, useState } from 'react'
import { useMutation, useQuery } from '@tanstack/react-query'
import { ipc, PROTOCOL_LABEL, type Protocol, type SavedKey } from '../lib/ipc'
import { unusableReason } from './setup-sheet'
import { CheckIcon, CloseIcon } from './icons'

/** What a tool the app cannot set up needs: an address per protocol, a key and model names. Nothing is written. */
export function ManualSheet({ keys, keysLoading, onClose }: { keys: SavedKey[]; keysLoading: boolean; onClose: () => void }) {
  const usable = keys.filter(key => !unusableReason(key))
  const [keyId, setKeyId] = useState('')
  useEffect(() => { if (!keyId && usable[0]) setKeyId((usable.find(key => key.is_default) ?? usable[0]).id) }, [keyId, usable])
  const key = keys.find(item => item.id === keyId)
  const endpoints = useQuery({ queryKey: ['endpoints'], queryFn: ipc.endpoints })
  const [copied, setCopied] = useState<string | null>(null)
  const copy = useMutation({
    mutationFn: async ({ id, text }: { id: string; text?: string }) => { await (text === undefined ? ipc.copyKey(keyId) : ipc.copyText(text)); return id },
    onSuccess: id => { setCopied(id); setTimeout(() => setCopied(current => current === id ? null : current), 1500) },
  })

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape') onClose() }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])

  const CopyButton = ({ id, text, label = '复制' }: { id: string; text?: string; label?: string }) =>
    <button className="btn" style={{ flexShrink: 0 }} disabled={copy.isPending} onClick={() => copy.mutate({ id, text })}>{copied === id ? <><CheckIcon size={14} />已复制</> : label}</button>

  return <div className="scrim" onMouseDown={event => { if (event.target === event.currentTarget) onClose() }}>
    <section className="sheet" role="dialog" aria-modal="true" aria-labelledby="manual-title">
      <div className="sheet-head">
        <div className="glyph" style={{ width: 40, height: 40, borderRadius: 11, fontSize: 14 }}>+</div>
        <div style={{ flex: 1 }}>
          <h2 id="manual-title" style={{ margin: 0, fontSize: 20, fontWeight: 600, letterSpacing: '-.02em' }}>手动接入其他客户端</h2>
          <div style={{ fontSize: 13, color: 'var(--muted)', marginTop: 3 }}>把下面三项填进客户端设置，这里不改任何文件</div>
        </div>
        <button className="btn icon-btn btn-ghost" aria-label="关闭" onClick={onClose}><CloseIcon /></button>
      </div>
      <div className="sheet-body">
        <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
          <span className="label">地址</span>
          {(endpoints.data ?? []).map(endpoint => <div key={endpoint.protocol} className="option" style={{ cursor: 'default' }}>
            <span style={{ width: 92, fontSize: 13, color: 'var(--muted)' }}>{PROTOCOL_LABEL[endpoint.protocol]}</span>
            <span className="mono" style={{ flex: 1, minWidth: 0, fontSize: 13, overflow: 'hidden', textOverflow: 'ellipsis', userSelect: 'text' }}>{endpoint.base_url}</span>
            <CopyButton id={`url:${endpoint.protocol}`} text={endpoint.base_url} />
          </div>)}
          {endpoints.error && <div className="banner banner-error">{endpoints.error.message}</div>}
          <div style={{ fontSize: 12, color: 'var(--soft)' }}>不确定选哪个时，用 OpenAI 地址</div>
        </div>

        <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
          <span className="label">KEY</span>
          <>
            {keysLoading && <div className="empty">正在读取 Key…</div>}
            {!keysLoading && usable.length === 0 && <div className="empty">没有可用的 Key，请先在「Key」页添加</div>}
            <div role="radiogroup" aria-label="Key" style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
              {usable.length > 0 && <div className="key-group">{usable.map(item => <button key={item.id} role="radio" aria-checked={keyId === item.id} className="option" onClick={() => setKeyId(item.id)}>
                <span className="radio" />
                <span style={{ flex: 1, fontSize: 14, fontWeight: 500 }}>{item.name}</span>
                <span className="chip">{item.key_prefix}…</span>
              </button>)}</div>}
            </div>
            {key && <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
              <span style={{ flex: 1, fontSize: 12, color: 'var(--soft)' }}>只复制到剪贴板，不在窗口显示</span>
              <CopyButton id="key" label="复制 Key" />
            </div>}
          </>
          {copy.error && <div className="banner banner-error" role="alert">{copy.error.message}</div>}
        </div>

        {key && <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
          <span className="label">模型</span>
          {(Object.keys(PROTOCOL_LABEL) as Protocol[]).map(protocol => {
            const models = key.models[protocol] ?? []
            if (models.length === 0) return null
            return <div key={protocol} style={{ display: 'flex', gap: 6, flexWrap: 'wrap', alignItems: 'center' }}>
              <span style={{ width: 92, fontSize: 13, color: 'var(--muted)' }}>{PROTOCOL_LABEL[protocol]}</span>
              {models.map(model => <button key={model} className="chip" style={{ cursor: 'pointer' }} title="点击复制" onClick={() => copy.mutate({ id: `model:${protocol}:${model}`, text: model })}>
                {copied === `model:${protocol}:${model}` ? '已复制' : model}
              </button>)}
            </div>
          })}
          <div style={{ fontSize: 12, color: 'var(--soft)' }}>也可以填列表外的模型名</div>
        </div>}
      </div>
      <div className="sheet-foot">
        <span style={{ flex: 1 }} />
        <button className="btn btn-primary" onClick={onClose}>完成</button>
      </div>
    </section>
  </div>
}
