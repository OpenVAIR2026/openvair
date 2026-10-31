import { useMutation, useQuery } from '@tanstack/react-query'
import { api, formatShortDate } from '../lib/api'
import { queryClient, useUser } from '../lib/queries'
import { ErrorBox } from './common'
import { Button } from './ui/button'

interface ClientToken {
  id: string
  client_name: string
  created_at: string
  last_seen_at: string | null
  expires_at: string
}

/** VAIR Desktop installs signed in through the device grant. */
export function ClientTokensSetting() {
  const user = useUser()
  const queryKey = ['client-tokens', user.user_id]
  const tokens = useQuery({ queryKey, queryFn: ({ signal }) => api<ClientToken[]>('/me/client-tokens', { signal }) })
  const revoke = useMutation({
    mutationFn: (id: string) => api(`/me/client-tokens/${id}`, { method: 'DELETE' }),
    onSettled: () => queryClient.invalidateQueries({ queryKey }),
  })

  return <div className="settings-item settings-sessions-item">
    <div className="settings-item-content">
      <span className="settings-item-label">已授权的客户端</span>
      {tokens.isPending ? <p className="settings-item-value">加载中…</p>
        : tokens.data?.length === 0 ? <p className="settings-field-hint">还没有授权 VAIR Desktop。</p>
        : tokens.data && <ul className="settings-session-list" aria-label="已授权的客户端">
          {tokens.data.map(token => <li key={token.id} className="settings-session">
            <div className="min-w-0">
              <div className="text-sm">{token.client_name}</div>
              <p className="settings-field-hint">最近使用 {formatShortDate(token.last_seen_at ?? token.created_at)} · 授权于 {formatShortDate(token.created_at)}</p>
            </div>
            <Button type="button" variant="ghost" size="sm" disabled={revoke.isPending} aria-label={`吊销 ${token.client_name}`} onClick={() => revoke.mutate(token.id)}>吊销</Button>
          </li>)}
        </ul>}
      {!!tokens.data?.length && <p className="settings-field-hint">吊销后客户端需要重新登录；已写入 Agent 的 Key 不受影响。</p>}
      <ErrorBox error={tokens.error ?? revoke.error} />
    </div>
  </div>
}
