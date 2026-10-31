import { useEffect, useState } from 'react'
import { useSearchParams } from 'react-router-dom'
import { useMutation, useQuery } from '@tanstack/react-query'
import { LoaderCircle, Pencil, Plus, Power, ShieldCheck, Trash2 } from 'lucide-react'
import { api, formatDate, formatShortDate, jsonBody, type Channel, type ChannelConcurrency } from '../lib/api'
import { queryClient, refreshModels, useChannels, useUser } from '../lib/queries'
import { Button } from '../components/ui/button'
import { Badge, Confirm, DataTable, Empty, ErrorBox, Health, Loading, PageHeader } from '../components/common'
import { ChannelEditor } from '../components/channel-editor'
import { ModelScope } from '../components/model-tags'
import { SourceStatus, VairChannelEditor } from '../components/vair-key-channels'

function ChannelHealth({ channel, checking }: { channel: Channel; checking: boolean }) {
  // A source key that cannot serve requests now matters more than the last check result.
  if (channel.kind === 'vair_key' && (!channel.enabled || channel.source?.status !== 'active')) return <SourceStatus source={channel.source} enabled={channel.enabled} />
  if (checking) return <span className="inline-flex items-center gap-1.5 rounded-md bg-warning/10 px-2 py-1 text-xs font-medium text-warning"><LoaderCircle aria-hidden="true" className="size-3.5 animate-spin" />检查中…</span>
  return <div className="grid justify-items-start gap-1">
    <Health enabled={channel.enabled} status={channel.health_status} errorKind={channel.health_error_kind} detail={channel.health_status !== 'healthy' ? channel.health_error : null} note={channel.health_note} />
    {channel.health_checked_at && <span className="text-[11px] tabular-nums text-muted-foreground" title={formatDate(channel.health_checked_at)}>{formatShortDate(channel.health_checked_at)}</span>}
  </div>
}

const host = (url: string) => { try { return new URL(url).host } catch { return url } }

export function ChannelsPage() {
  const user = useUser()
  const query = useChannels()
  const [searchParams, setSearchParams] = useSearchParams()
  // `?add=1` opens the editor directly, e.g. from the overview's getting-started steps.
  const [open, setOpen] = useState(() => searchParams.has('add'))
  useEffect(() => { if (searchParams.has('add')) setSearchParams(previous => { const next = new URLSearchParams(previous); next.delete('add'); return next }, { replace: true }) }, [searchParams, setSearchParams])
  const [edit, setEdit] = useState<Channel | null>(null)
  const capped = query.data?.some(channel => channel.concurrency_limit) ?? false
  const concurrency = useQuery({ queryKey: ['channel-concurrency', user.account_id], queryFn: ({ signal }) => api<ChannelConcurrency[]>('/channels/concurrency', { signal }), enabled: capped, refetchInterval: 3000 })
  const inUse = new Map(concurrency.data?.map(item => [item.channel_id, item.in_use]))
  const [remove, setRemove] = useState<Channel | null>(null)
  const refresh = async () => {
    await refreshModels()
    await queryClient.invalidateQueries({ queryKey: ['routes', user.account_id] })
  }
  const deletion = useMutation({ mutationFn: (id: string) => api(`/channels/${id}`, { method: 'DELETE' }), onSuccess: async () => { setRemove(null); await refresh() } })
  const toggle = useMutation({ mutationFn: ({ id, enabled }: { id: string; enabled: boolean }) => api(`/channels/${id}`, jsonBody({ enabled }, 'PATCH')), onSuccess: refresh })
  const check = useMutation({ mutationFn: (id: string) => api<{ healthy: boolean; message: string }>(`/channels/${id}/check`, { method: 'POST' }), onSuccess: async () => { await refresh(); await queryClient.invalidateQueries({ queryKey: ['monitoring-runs', user.account_id] }) } })
  const show = () => { check.reset(); setOpen(true) }
  const showEditor = (channel: Channel) => { check.reset(); setEdit(channel) }
  return <><PageHeader title="上游渠道" description="接入已购买或获授权的 API。模型列表支持手动填写或一键获取，兼容渠道也可以全部转发。" action=<Button onClick={show}><Plus aria-hidden="true" />添加渠道</Button> /><ErrorBox error={query.error} retry={() => query.refetch()} /><ErrorBox error={toggle.error} /><ErrorBox error={check.error} />{query.isPending ? <Loading /> : !query.isError && <DataTable data={query.data ?? []} columns={[
    { accessorKey: 'name', header: '渠道', cell: ({ row }) => <div className="grid gap-0.5">
      <span className="inline-flex items-center gap-1.5 font-medium">{row.original.name}{row.original.kind === 'vair_key' && <Badge>VAIR</Badge>}</span>
      {row.original.kind === 'vair_key' ? <span className="block max-w-[220px] truncate text-xs text-muted-foreground">来自 {row.original.source?.account_name ?? '已撤销的 Key'}</span> : <span className="block max-w-[220px] truncate text-xs text-muted-foreground" title={row.original.base_url}>{host(row.original.base_url)}</span>}
    </div> },
    { accessorKey: 'protocol', header: '协议', cell: ({ row }) => <Badge>{row.original.protocol === 'any' ? '全部协议' : row.original.protocol}</Badge> },
    { accessorKey: 'priority', header: '优先级' },
    { accessorKey: 'concurrency_limit', header: '并发', cell: ({ row }) => {
      const limit = row.original.concurrency_limit
      if (!limit) return <span className="text-xs text-muted-foreground">不限</span>
      const used = inUse.get(row.original.id) ?? 0
      return <span className={`tabular-nums ${used >= limit ? 'font-medium text-destructive' : ''}`} title={used >= limit ? '已满，新请求会改用下一个渠道' : undefined}>{used} / {limit}</span>
    } },
    { accessorKey: 'model_count', header: '模型范围', cell: ({ row }) => row.original.kind === 'vair_key' ? <ModelScope rules={row.original.source?.allowed_models ?? []} mode={row.original.source?.model_mode ?? 'all'} known={row.original.source?.models ?? []} kind="当前可用" /> : <ModelScope rules={row.original.models} mode={row.original.model_mode} known={row.original.known_models} /> },
    { accessorKey: 'health_status', header: '健康状态', cell: ({ row }) => <ChannelHealth channel={row.original} checking={check.isPending && check.variables === row.original.id} /> },
    { id: 'actions', header: '操作', cell: ({ row }) => <div className="flex items-center gap-1"><Button variant="ghost" size="sm" disabled={check.isPending || toggle.isPending} onClick={() => check.mutate(row.original.id)}>{check.isPending && check.variables === row.original.id ? <LoaderCircle aria-hidden="true" className="size-4 animate-spin" /> : <ShieldCheck aria-hidden="true" className="size-4" />}{check.isPending && check.variables === row.original.id ? '检查中…' : '检查'}</Button><Button variant="ghost" size="icon" aria-label={`${row.original.enabled ? '停用' : '启用'}渠道 ${row.original.name}`} title={row.original.enabled ? '停用' : '启用'} disabled={toggle.isPending} onClick={() => toggle.mutate({ id: row.original.id, enabled: !row.original.enabled })}><Power className={row.original.enabled ? 'size-4' : 'size-4 text-success'} /></Button><Button variant="ghost" size="sm" onClick={() => showEditor(row.original)}><Pencil className="size-4" />编辑</Button><Button variant="ghost" size="icon" aria-label={`删除渠道 ${row.original.name}`} onClick={() => { deletion.reset(); setRemove(row.original) }}><Trash2 className="size-4 text-destructive" /></Button></div> },
  ]} empty={<Empty title="接入第一个上游渠道" description="支持 OpenAI、Anthropic、Gemini 原生及兼容协议。上游 API Key 加密存储。" action=<Button variant="outline" onClick={show}>添加渠道</Button> />} />}
  {edit?.kind === 'vair_key'
    ? <VairChannelEditor key={edit.id} channel={edit} onClose={() => setEdit(null)} onSaved={async () => { check.reset(); await refresh(); await queryClient.invalidateQueries({ queryKey: ['monitoring-runs', user.account_id] }) }} />
    : (open || edit) && <ChannelEditor key={edit?.id ?? 'create'} channel={edit ?? undefined} onClose={() => { setOpen(false); setEdit(null) }} onSaved={async () => { check.reset(); await refresh(); await queryClient.invalidateQueries({ queryKey: ['monitoring-runs', user.account_id] }) }} />}
  <Confirm open={!!remove} onOpenChange={value => { if (!value && !deletion.isPending) setRemove(null) }} title="删除渠道？" description={remove?.kind === 'vair_key' ? `删除「${remove.name}」后不再通过这把 VAIR Key 转发请求。` : `「${remove?.name ?? ''}」及其密钥和路由将被删除。`} pending={deletion.isPending} error={deletion.error} onConfirm={() => { if (remove) deletion.mutate(remove.id) }} />
  </>
}
