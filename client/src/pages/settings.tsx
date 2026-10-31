import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { ipc, RELEASES_URL, type ServerChange, type Session } from '../lib/ipc'
import { BiliCard } from '../components/bili-card'
import { useAutostart } from '../lib/autostart'

export function SettingsPage({ session }: { session: Session }) {
  const info = useQuery({ queryKey: ['app-info'], queryFn: ipc.appInfo })

  return <main className="main">
    <h1 className="h1">设置</h1>
    <div className="grid-2" style={{ paddingBottom: 24 }}>
      <div className="stack">
        <ServerCard session={session} />
        <BiliCard />
      </div>

      <div className="stack">
        <div className="card">
          <div className="card-head">本机</div>
          <AutostartItem />
          <SyncItem />
          <RefreshModelsItem />
          <div className="item">
            <div style={{ flex: 1, minWidth: 0 }}><div className="item-k">配置备份</div><div className="mono item-s" style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{info.data?.backups_dir ?? '…'}</div></div>
            <button className="btn" onClick={() => ipc.openBackups()}>打开文件夹</button>
          </div>
        </div>
        <div className="card">
          <div className="card-head">关于</div>
          <div className="item">
            <div style={{ flex: 1 }}><div className="item-k">OpenVAIR Desktop <span className="mono">{info.data?.version}</span></div><div className="item-s">开源版不自动更新，新版本在 GitHub Releases 发布</div></div>
            <button className="btn" onClick={() => ipc.openExternal(RELEASES_URL)}>查看新版本 ↗</button>
          </div>
        </div>
      </div>
    </div>
  </main>
}

function SyncItem() {
  const queryClient = useQueryClient()
  const status = useQuery({ queryKey: ['sync-setting'], queryFn: ipc.syncSetting })
  const change = useMutation({
    mutationFn: ipc.setSyncSetting,
    onSuccess: on => { queryClient.setQueryData(['sync-setting'], on); queryClient.invalidateQueries({ queryKey: ['agents'] }) },
  })
  const on = status.data
  return <div className="item">
    <div style={{ flex: 1, minWidth: 0 }}>
      <div className="item-k">自动更新 Agent 的模型列表</div>
      <div className="item-s">{change.error ? change.error.message : 'Key 的模型有增减时，更新本机写入的 /model 菜单和 Codex 模型目录；不改动所选模型和其他设置'}</div>
    </div>
    <button className="switch" role="switch" aria-label="自动更新 Agent 的模型列表" aria-checked={!!on}
      disabled={on === undefined || change.isPending} onClick={() => change.mutate(!on)}><span /></button>
  </div>
}

/** Syncs now, with automatic syncing on or off. */
function RefreshModelsItem() {
  const queryClient = useQueryClient()
  const refresh = useMutation({
    mutationFn: ipc.refreshModels,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['agents'] }),
  })
  const status = refresh.isPending ? '正在更新…'
    : refresh.error ? refresh.error.message
    : refresh.data ? (refresh.data.length ? `已更新 ${refresh.data.join('、')}` : '已是最新')
    : '从服务器读取 Key 的模型，并按本机 Codex 的版本重建模型目录'
  return <div className="item">
    <div style={{ flex: 1, minWidth: 0 }}>
      <div className="item-k">立即更新模型列表</div>
      <div className="item-s">{status}</div>
    </div>
    <button className="btn" disabled={refresh.isPending} onClick={() => refresh.mutate()}>立即更新</button>
  </div>
}

function AutostartItem() {
  const { on, change } = useAutostart()
  return <div className="item">
    <div style={{ flex: 1, minWidth: 0 }}>
      <div className="item-k">开机启动</div>
      <div className="item-s">{change.error ? change.error.message : '登录后在托盘中运行，保持 bili 可用；关闭窗口不会退出 OpenVAIR Desktop'}</div>
    </div>
    <button className="switch" role="switch" aria-label="开机启动" aria-checked={!!on}
      disabled={on === undefined || change.isPending} onClick={() => change.mutate(!on)}><span /></button>
  </div>
}

/** One server per computer. A new address of the same server keeps the keys; another server starts over. */
function ServerCard({ session }: { session: Session }) {
  const queryClient = useQueryClient()
  const keys = useQuery({ queryKey: ['keys'], queryFn: ipc.keys })
  const [address, setAddress] = useState<string | null>(null)
  // A public plain-HTTP address waiting for the user to confirm.
  const [warning, setWarning] = useState<string | null>(null)
  const [done, setDone] = useState<ServerChange | null>(null)
  const current = session.server ? origin(session.server) : null
  const announced = keys.data?.map(key => key.summary?.server_url).find(url => !!url && origin(url) !== current) ?? null
  const move = useMutation({
    mutationFn: async (allow: boolean) => {
      const check = await ipc.checkServer(address?.trim() ?? '')
      if (check.public_http && !allow) {
        setWarning(check.server)
        return null
      }
      return ipc.changeServer(check.server, check.public_http)
    },
    onSuccess: async change => {
      if (!change) return
      close()
      setDone(change)
      await queryClient.invalidateQueries()
    },
  })
  const discard = useMutation({ mutationFn: () => ipc.changeServer(null, false, true), onSuccess: () => queryClient.invalidateQueries() })
  const close = () => { setAddress(null); setWarning(null); move.reset(); discard.reset() }
  const open = (value: string) => { setDone(null); setAddress(value) }
  return <div className="card">
    <div className="card-head">服务器</div>
    <div className="item">
      <div style={{ flex: 1, minWidth: 0 }}>
        <div className="item-k mono">{session.server}</div>
        <div className="item-s">{session.public_http ? '通过公网使用 http://，Key 和请求明文传输' : session.plain_http ? '使用 http://，Key 和请求在局域网中不加密传输' : '本机所有 Key 都来自这台 OpenVAIR 服务器'}</div>
      </div>
      {address === null && <button className="btn" onClick={() => open('')}>更换服务器</button>}
    </div>
    {announced && address === null && <div className="banner banner-warn" style={{ alignItems: 'center', gap: 12, margin: '0 18px 16px' }}>
      <div style={{ flex: 1 }}>服务器管理员把访问地址设成了 <span className="mono">{announced}</span>，改用后 Key 和已接入的 Agent 都会保留。</div>
      <button className="btn" onClick={() => open(announced)}>改用新地址</button>
    </div>}
    {done && <div className="banner banner-ok" role="status" style={{ margin: '0 18px 16px' }}><div>
      已改用 <span className="mono">{done.server}</span>。{done.updated.length > 0 && `${done.updated.join('、')} 已指向新地址。`}
      {done.skipped.length > 0 && `${done.skipped.join('、')} 的配置文件已被改动，没有更新，请重新接入。`}
    </div></div>}
    {address !== null && <form className="item" style={{ flexDirection: 'column', alignItems: 'stretch', gap: 10 }}
      onSubmit={event => { event.preventDefault(); if (address.trim()) move.mutate(false) }}>
      <label className="label" htmlFor="new-server">新地址</label>
      <input id="new-server" className="input mono" value={address} onChange={event => { setAddress(event.target.value); setWarning(null); move.reset() }}
        placeholder="https://openvair.example.com" autoFocus spellCheck={false} autoComplete="off" />
      <div className="item-s">同一台服务器换了 IP 或域名时，本机的 {session.key_count} 把 Key 都会保留，已接入的 Agent 会改用新地址。要换到另一台服务器，需要先移除这些 Key。</div>
      {move.error && <div className="banner banner-error" role="alert">{move.error.message}</div>}
      {warning && <div className="banner banner-warn" role="alert" style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
        <div><span className="mono">{warning}</span> 是公网地址，使用 http:// 时 Key 和对话内容会明文经过网络，途中任何人都能看到并盗用 Key。建议给服务器配上 HTTPS。</div>
        <div style={{ display: 'flex', gap: 8 }}>
          <button className="btn btn-danger" type="button" disabled={move.isPending} onClick={() => move.mutate(true)}>仍然使用 http://</button>
          <button className="btn btn-ghost" type="button" onClick={() => setWarning(null)}>返回修改</button>
        </div>
      </div>}
      {discard.error && <div className="banner banner-error" role="alert">{discard.error.message}</div>}
      <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
        <button className="btn btn-primary" type="submit" disabled={!address.trim() || move.isPending || !!warning}>{move.isPending ? '正在验证 Key…' : '更换地址'}</button>
        <button className="btn btn-ghost" type="button" onClick={close}>取消</button>
        <button className="btn btn-ghost" type="button" style={{ marginLeft: 'auto' }} disabled={discard.isPending}
          onClick={() => discard.mutate()}>换到另一台服务器（移除全部 Key）</button>
      </div>
    </form>}
  </div>
}

const origin = (address: string) => {
  try { return new URL(address).origin } catch { return address }
}
