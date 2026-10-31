import { useState } from 'react'
import { useMutation, useQuery } from '@tanstack/react-query'
import { api, formatShortDate, jsonBody } from '../lib/api'
import { queryClient, useUser } from '../lib/queries'
import { Badge, ErrorBox } from './common'
import { Button } from './ui/button'

interface Session {
  id: string
  created_at: string
  last_seen_at: string | null
  expires_at: string
  user_agent: string | null
  current: boolean
}

/** A readable device name from the browser's user agent, e.g. "Chrome · macOS". */
export function describeDevice(userAgent: string | null) {
  if (!userAgent) return '未知设备'
  const browser = /Edg\//.test(userAgent) ? 'Edge'
    : /OPR\//.test(userAgent) ? 'Opera'
    : /Firefox\//.test(userAgent) ? 'Firefox'
    : /Chrome\//.test(userAgent) ? 'Chrome'
    : /Safari\//.test(userAgent) ? 'Safari'
    : ''
  const system = /iPhone|iPad/.test(userAgent) ? 'iOS'
    : /Android/.test(userAgent) ? 'Android'
    : /Windows/.test(userAgent) ? 'Windows'
    : /Mac OS X/.test(userAgent) ? 'macOS'
    : /Linux/.test(userAgent) ? 'Linux'
    : ''
  return [browser, system].filter(Boolean).join(' · ') || '其他设备'
}

export function SessionsSetting() {
  const user = useUser()
  const queryKey = ['sessions', user.user_id]
  const sessions = useQuery({ queryKey, queryFn: ({ signal }) => api<Session[]>('/me/sessions', { signal }) })
  const [notice, setNotice] = useState('')
  const refresh = () => queryClient.invalidateQueries({ queryKey })
  const revoke = useMutation({ mutationFn: (id: string) => api(`/me/sessions/${id}`, { method: 'DELETE' }), onMutate: () => setNotice(''), onSettled: refresh })
  const revokeOthers = useMutation({
    mutationFn: () => api<{ revoked: number }>('/me/sessions/revoke-others', jsonBody({})),
    onMutate: () => setNotice(''),
    onSuccess: result => setNotice(`已退出 ${result.revoked} 个其他设备。`),
    onSettled: refresh,
  })
  const others = (sessions.data ?? []).filter(session => !session.current).length
  const busy = revoke.isPending || revokeOthers.isPending

  return <div className="settings-item settings-sessions-item">
    <div className="settings-item-content">
      <span className="settings-item-label">登录设备</span>
      {sessions.isPending ? <p className="settings-item-value">加载中…</p>
        : sessions.data && <ul className="settings-session-list" aria-label="登录设备">
          {sessions.data.map(session => {
            const device = describeDevice(session.user_agent)
            return <li key={session.id} className="settings-session">
              <div className="min-w-0">
                <div className="flex flex-wrap items-center gap-2 text-sm">{device}{session.current && <Badge tone="success">当前设备</Badge>}</div>
                <p className="settings-field-hint">最近活动 {formatShortDate(session.last_seen_at ?? session.created_at)} · 登录于 {formatShortDate(session.created_at)}</p>
              </div>
              {!session.current && <Button type="button" variant="ghost" size="sm" disabled={busy} aria-label={`退出 ${device} 的登录`} onClick={() => revoke.mutate(session.id)}>退出</Button>}
            </li>
          })}
        </ul>}
      {notice && <p className="settings-field-hint" role="status">{notice}</p>}
      <ErrorBox error={sessions.error ?? revoke.error ?? revokeOthers.error} />
    </div>
    <div className="settings-item-actions">
      <Button type="button" variant="outline" size="sm" disabled={busy || others === 0} onClick={() => revokeOthers.mutate()}>{revokeOthers.isPending ? '退出中…' : '退出其他设备'}</Button>
    </div>
  </div>
}
