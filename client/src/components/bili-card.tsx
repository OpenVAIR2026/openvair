import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { ipc, type BiliAgent, type BiliStatus } from '../lib/ipc'
import { useAutostart } from '../lib/autostart'
import { CheckIcon, WarnIcon } from './icons'

const VERSION = '0.1.197'
const NODE_URL = 'https://nodejs.org/'

const AGENTS: { id: BiliAgent; key: 'claude_code' | 'codex'; name: string; note: string }[] = [
  { id: 'claude-code', key: 'claude_code', name: 'Claude Code', note: '对新开的 Claude Code 对话生效' },
  { id: 'codex', key: 'codex', name: 'Codex', note: '对新开的 Codex 对话生效；OpenVAIR 在托盘中运行时会保持 bili 运行' },
]

/** bili context compression for Claude Code and Codex: install the component once, then switch it on or off per agent. */
export function BiliCard() {
  const queryClient = useQueryClient()
  const agents = useQuery({ queryKey: ['agents'], queryFn: ipc.agents })
  // The app checks the proxy every 30 seconds; follow it while an agent uses bili.
  const status = useQuery({ queryKey: ['bili'], queryFn: ipc.biliStatus, refetchInterval: query => query.state.data?.running == null ? false : 10000 })
  const autostart = useAutostart()
  const done = (value: BiliStatus) => queryClient.setQueryData(['bili'], value)
  const install = useMutation({ mutationFn: ipc.installBili, onSuccess: done })
  const toggle = useMutation({
    mutationFn: ({ agent, on }: { agent: BiliAgent; on: boolean }) => on ? ipc.enableBili(agent) : ipc.disableBili(agent),
    onSuccess: done,
  })
  const bili = status.data
  const open = useMutation({ mutationFn: ipc.openBili })
  const error = install.error ?? toggle.error ?? open.error ?? autostart.change.error ?? status.error
  const enabled = AGENTS.some(agent => bili?.[agent.key].enabled)

  const proxy = bili?.error ? '异常'
    : bili?.running === true ? '运行中'
    : '未运行，OpenVAIR 正在重新启动'
  // Mutation errors first; otherwise why the proxy is unhealthy.
  const notice = error?.message ?? (enabled ? bili?.error : null)
  const component = !bili ? '正在检查…'
    : bili.installed ? <span style={{ color: 'var(--ok)', display: 'inline-flex', alignItems: 'center', gap: 4 }}><CheckIcon size={13} />已安装</span>
    : install.isPending ? '正在安装…'
    : bili.npm ? '未安装' : '需要 Node.js 18+'

  return <div className="card">
    <div className="card-head">上下文压缩</div>
    {AGENTS.map(agent => {
      const connected = !!agents.data?.some(item => item.id === agent.id && item.connection)
      const usable = connected && !!bili?.installed
      const on = !!bili?.[agent.key].enabled
      const pending = toggle.isPending && toggle.variables?.agent === agent.id
      const hint = !connected ? `需先接入 ${agent.name}`
        : bili && !bili.installed ? '需先安装 bili 组件'
        : pending ? (toggle.variables?.on ? '正在开启…' : '正在关闭…')
        : toggle.isSuccess && toggle.variables?.agent === agent.id && on ? agent.note
        : null
      return <div className="item" key={agent.id}>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div className="item-k">{agent.name} 长对话压缩</div>
          <div className="item-s">自动压缩 {agent.name} 长对话的上下文，减少 token 消耗{hint && ` · ${hint}`}</div>
        </div>
        <button className="switch" role="switch" aria-label={`${agent.name} 长对话压缩`} aria-checked={on}
          disabled={!usable || toggle.isPending} style={{ opacity: usable ? 1 : 0.4 }}
          onClick={() => toggle.mutate({ agent: agent.id, on: !on })}><span /></button>
      </div>
    })}
    <div className="item">
      <div style={{ flex: 1, minWidth: 0 }}>
        <div className="item-k">bili 组件 <span className="mono item-s">{VERSION}</span></div>
        <div className="item-s">{component}{enabled && bili?.installed && ` · 代理${proxy}`}</div>
      </div>
      {bili && !bili.installed && (bili.npm
        ? <button className="btn" disabled={install.isPending} onClick={() => install.mutate()}>{install.isPending ? '安装中…' : '安装'}</button>
        : <button className="btn" onClick={() => ipc.openExternal(NODE_URL)}>下载 Node.js ↗</button>)}
      {enabled && <button className="btn" disabled={open.isPending} onClick={() => open.mutate()}>打开 bili 界面 ↗</button>}
    </div>
    {bili?.codex.enabled && autostart.on === false && <div className="item">
      <div style={{ flex: 1, minWidth: 0 }} className="item-s">重启电脑后需要先打开 OpenVAIR，Codex 才能连接</div>
      <button className="btn" disabled={autostart.change.isPending} onClick={() => autostart.change.mutate(true)}>开启开机启动</button>
    </div>}
    {notice && <div className="item"><div className="banner banner-error" role="alert" style={{ flex: 1, whiteSpace: 'pre-wrap' }}><WarnIcon />{notice}</div></div>}
  </div>
}
