import { HOLDER_STATUS, relativeTime, type Agent, type SavedKey } from './ipc'

export type Tone = 'ok' | 'warn' | 'idle' | 'off'

export interface AgentState {
  tone: Tone
  label: string
  /** The main action on the card. */
  action: 'connect' | 'switch' | 'install' | null
  key: SavedKey | undefined
}

/** One sentence for each agent card, from the local config and the saved keys. */
export function describeAgent(agent: Agent, keys: SavedKey[] | undefined): AgentState {
  const connection = agent.connection
  const key = connection ? keys?.find(item => item.id === connection.key_id) : undefined
  if (agent.config_error) return { tone: 'warn', label: agent.config_error, action: null, key }
  if (!connection) {
    if (agent.status === 'other') return { tone: 'idle', label: '已指向 OpenVAIR，但不是在本机配置的', action: 'connect', key }
    if (!agent.installed) return { tone: 'off', label: '未检测到', action: 'install', key }
    return { tone: 'idle', label: '已安装，尚未接入 OpenVAIR', action: 'connect', key }
  }
  if (keys && !key) return { tone: 'warn', label: '使用的 Key 已从本机移除', action: 'switch', key }
  if (key?.invalid) return { tone: 'warn', label: 'Key 已失效或被撤销', action: 'switch', key }
  if (key?.summary && key.summary.status !== 'active') return { tone: 'warn', label: `Key ${HOLDER_STATUS[key.summary.status].label}`, action: 'switch', key }
  if (agent.status === 'edited') return { tone: 'warn', label: '配置文件已被手动修改', action: 'switch', key }
  const test = connection.last_test
  if (test && !test.ok) return { tone: 'warn', label: test.message, action: 'switch', key }
  if (key?.error) return { tone: 'warn', label: '无法连接服务器', action: 'switch', key }
  return { tone: 'ok', label: test ? `正常 · ${relativeTime(test.checked_at)}` : '已接入', action: 'switch', key }
}
