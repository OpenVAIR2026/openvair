import { useEffect, useState, type FormEvent } from 'react'
import { useMutation } from '@tanstack/react-query'
import { LoaderCircle } from 'lucide-react'
import { Link, useLocation, useNavigate } from 'react-router-dom'
import { api, formatDate, jsonBody, type SharedKey } from '../lib/api'
import { queryClient, useUser } from '../lib/queries'
import { Button } from '../components/ui/button'
import { Input } from '../components/ui/input'
import { ErrorBox, Field, PageHeader, Panel } from '../components/common'
import { SourceKeyDetails } from '../components/vair-key-channels'

/**
 * Accepts a key shared through `/share#<code>`. The code lives in the fragment
 * so it never reaches server logs, and is spent once the channel is added.
 */
export function SharePage() {
  const user = useUser(); const navigate = useNavigate()
  const code = decodeURIComponent(useLocation().hash.slice(1)).trim()
  const [name, setName] = useState(''); const [priority, setPriority] = useState('100')
  const resolve = useMutation({ mutationFn: () => api<SharedKey>('/key-shares/resolve', jsonBody({ code })), onSuccess: source => setName(current => current || `${source.account_name} · ${source.key_name}`) })
  const accept = useMutation({ mutationFn: () => api<{ id: string }>('/key-shares/accept', jsonBody({ code, name: name.trim(), priority: Number(priority) })), onSuccess: async () => { await queryClient.invalidateQueries({ queryKey: ['channels', user.account_id] }); navigate('/channels', { replace: true }) } })
  useEffect(() => { if (code) resolve.mutate() }, [code])
  const submit = (event: FormEvent<HTMLFormElement>) => { event.preventDefault(); if (resolve.data && name.trim() && !accept.isPending) accept.mutate() }
  return <>
    <PageHeader title="接受分享的 Key" description="对方的 Key 会作为渠道加入你的帐号，请求按它的模型和额度转发，用量计入对方的 Key。你看不到 Key 明文，对方撤销后渠道随之失效。" />
    <Panel className="max-w-xl p-5">
      {!code ? <p className="text-sm text-muted-foreground">链接缺少分享码，请向对方确认完整链接。</p>
        : resolve.isPending ? <p role="status" className="flex items-center gap-1.5 text-sm text-muted-foreground"><LoaderCircle aria-hidden="true" className="size-4 animate-spin" />正在读取分享…</p>
        : resolve.isError ? <div className="grid gap-3"><ErrorBox error={resolve.error} /><p className="text-sm text-muted-foreground">分享链接只能使用一次，24 小时内有效。请让对方重新生成链接。</p><div><Button asChild variant="outline"><Link to="/channels">前往上游渠道</Link></Button></div></div>
        : resolve.data && <form className="grid gap-4" onSubmit={submit}>
          <SourceKeyDetails source={resolve.data} title="分享的 Key" action={<span className="text-xs text-muted-foreground">{formatDate(resolve.data.expires_at)} 前有效</span>} />
          <Field label="渠道名称">{id => <Input id={id} autoComplete="off" value={name} onChange={event => setName(event.target.value)} disabled={accept.isPending} />}</Field>
          <Field label="渠道优先级" hint="数值越小越优先。来源 Key 不可用时自动切换到其他渠道。">{id => <Input id={id} type="number" min="0" value={priority} onChange={event => setPriority(event.target.value)} disabled={accept.isPending} />}</Field>
          <ErrorBox error={accept.error} />
          <div className="flex justify-end gap-2"><Button asChild variant="outline"><Link to="/channels">取消</Link></Button><Button type="submit" disabled={accept.isPending || !name.trim()}>{accept.isPending ? '添加中…' : '添加为渠道'}</Button></div>
        </form>}
    </Panel>
  </>
}
