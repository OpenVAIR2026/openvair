import { useState } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { ipc, type SavedKey } from '../lib/ipc'
import { BrandMark } from '../components/icons'

/** The first screen: nothing works until a key is saved. */
export function WelcomePage() {
  return <main className="login">
    <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 18 }}>
      <div className="login-mark"><span className="ring" /><div className="tile"><BrandMark size={60} /></div></div>
      <div style={{ textAlign: 'center' }}>
        <h1 style={{ margin: 0, fontSize: 34, fontWeight: 600, letterSpacing: '-.035em' }}>一把 Key，接入本机所有 Agent</h1>
        <p style={{ margin: '10px 0 0', fontSize: 15, color: 'var(--muted)' }}>填入 OpenVAIR 服务器地址和管理员发给你的 Key，为 Codex、Claude Code、Gemini CLI 自动写好配置</p>
      </div>
    </div>
    <div className="login-card"><AddKeyForm /></div>
    <div style={{ fontSize: 12, color: 'var(--soft)' }}>Key 只保存在本机系统钥匙串，以后可以再添加这台服务器上的其他 Key</div>
  </main>
}

/** Checks a key against the server before saving it. The first key also sets the server; later ones go to the same one. */
export function AddKeyForm({ serverHost = null, onAdded, onCancel }: { serverHost?: string | null; onAdded?: (key: SavedKey) => void; onCancel?: () => void }) {
  const queryClient = useQueryClient()
  const [server, setServer] = useState('')
  const [secret, setSecret] = useState('')
  // The address the user agreed to send keys to over plain HTTP on the internet.
  const [allowed, setAllowed] = useState<string | null>(null)
  const [warning, setWarning] = useState<string | null>(null)
  const submit = useMutation({
    mutationFn: async (allow: boolean) => {
      if (serverHost) return ipc.addKey(null, secret.trim())
      const check = await ipc.checkServer(server.trim())
      if (check.public_http && !allow && allowed !== check.server) {
        setWarning(check.server)
        return null
      }
      return ipc.addKey(check.server, secret.trim(), check.public_http)
    },
    onSuccess: async key => {
      if (!key) return
      setSecret('')
      setWarning(null)
      await queryClient.invalidateQueries()
      onAdded?.(key)
    },
  })
  const ready = (!!serverHost || !!server.trim()) && !!secret.trim()
  return <form style={{ display: 'flex', flexDirection: 'column', gap: 12, width: '100%', maxWidth: 420 }} onSubmit={event => { event.preventDefault(); if (ready) submit.mutate(false) }}>
    {serverHost ? <div className="item-s">添加到 <span className="mono">{serverHost}</span> 上的 Key。要换服务器，请到「设置」里更换。</div> : <>
      <label className="label" htmlFor="key-server">服务器地址</label>
      <input id="key-server" className="input mono" value={server} onChange={event => { setServer(event.target.value); setWarning(null); submit.reset() }} placeholder="https://openvair.example.com" autoFocus spellCheck={false} autoComplete="off" />
    </>}
    <label className="label" htmlFor="key-secret">OPENVAIR KEY</label>
    <input id="key-secret" className="input mono" type="password" value={secret} onChange={event => { setSecret(event.target.value); submit.reset() }} placeholder="openvair_…" autoFocus={!!serverHost} spellCheck={false} autoComplete="off" />
    {submit.error && <div className="banner banner-error" role="alert">{submit.error.message}</div>}
    {warning && <div className="banner banner-warn" role="alert" style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
      <div><span className="mono">{warning}</span> 是公网地址，使用 http:// 时 Key 和对话内容会明文经过网络，途中任何人都能看到并盗用 Key。建议给服务器配上 HTTPS。</div>
      <div style={{ display: 'flex', gap: 8 }}>
        <button className="btn btn-danger" type="button" disabled={submit.isPending} onClick={() => { setAllowed(warning); submit.mutate(true) }}>仍然使用 http:// 添加</button>
        <button className="btn btn-ghost" type="button" onClick={() => setWarning(null)}>返回修改</button>
      </div>
    </div>}
    <div style={{ display: 'flex', gap: 10, justifyContent: 'center' }}>
      <button className="btn btn-primary btn-lg" type="submit" disabled={submit.isPending || !ready || !!warning}>{submit.isPending ? '验证中…' : '添加 Key'}</button>
      {onCancel && <button className="btn btn-lg btn-ghost" type="button" onClick={onCancel}>取消</button>}
    </div>
    {!serverHost && <div style={{ fontSize: 12, color: 'var(--soft)', textAlign: 'center' }}>建议使用 https://；局域网服务器可以用 http://，公网地址用 http:// 需要确认。客户端只能使用 Key、查看额度，不能修改或撤销 Key。</div>}
  </form>
}
