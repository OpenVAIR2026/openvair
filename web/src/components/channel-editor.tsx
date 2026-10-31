import { useEffect, useState } from 'react'
import { Controller, useFieldArray, useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import { useMutation } from '@tanstack/react-query'
import { ArrowRight, CheckCircle2, LoaderCircle, Plus, RefreshCw, ShieldCheck, Trash2 } from 'lucide-react'
import { api, jsonBody, type Channel, type Protocol } from '../lib/api'
import { useModelSuggestions, useProviders } from '../lib/queries'
import { channelSchema, channelEditSchema, channelConnectionFields, type ChannelValues } from '../lib/schemas'
import { isConcreteModel, splitModelTokens } from '../lib/model-rules'
import { Button } from './ui/button'
import { Checkbox } from './ui/checkbox'
import { Input } from './ui/input'
import { Choice, Confirm, ErrorBox, Field, FormSheet, SegmentedChoice } from './common'
import { ModelTagInput } from './model-tags'
import { ModelSelect } from './model-select'
import { VairKeyChannelForm } from './vair-key-channels'
import { CLOUD } from '../lib/edition'

const modelNames = (text: string) => [...new Set(splitModelTokens(text))].sort()
// The server accepts a check receipt for 15 minutes; leave a margin for the save request.
const CHECK_TTL_MS = 14 * 60 * 1000
const defaults: ChannelValues = { name: '', provider: 'custom', protocol: 'openai', base_url: '', secret: '', models: '', model_mode: 'allowlist', test_model: '', priority: '100', mappings: [] }
type Connection = { channel_id?: string; protocol: Protocol; base_url: string; secret?: string }
const protocolOptions: { value: Protocol; label: string }[] = [{ value: 'openai', label: 'OpenAI 兼容' }, { value: 'anthropic', label: 'Anthropic 兼容' }, { value: 'gemini', label: 'Gemini 兼容' }]
type CheckResult = { healthy: boolean; rate_limited?: boolean; message: string; check_receipt: string | null }
// Official and custom both create upstream channels; only official ones pick a vendor preset.
type Source = 'official' | 'custom' | 'vair_key'
const sourceOptions: { value: Source; label: string; description: string }[] = [
  { value: 'official', label: '官方 API Key', description: '选厂商，自动填地址' },
  { value: 'custom', label: '第三方 API Key', description: '自定义服务地址' },
  // Keys shared from another account need the hosted service's tenants.
  ...(CLOUD ? [{ value: 'vair_key' as const, label: 'VAIR Key', description: '来自其他用户共享' }] : []),
]

function connection(values: ChannelValues, id?: string): Connection {
  return { ...(id ? { channel_id: id } : {}), protocol: values.protocol, base_url: values.base_url.trim(), ...(values.secret.trim() ? { secret: values.secret.trim() } : {}) }
}
function initialValues(channel?: Channel): ChannelValues {
  // Only upstream channels reach this editor; VAIR key channels ('any') have their own.
  return channel ? { name: channel.name, provider: channel.provider ?? 'custom', protocol: channel.protocol === 'any' ? 'openai' : channel.protocol, base_url: channel.base_url, secret: '', models: channel.models.join('\n'), model_mode: channel.model_mode, test_model: channel.test_model ?? '', priority: String(channel.priority), mappings: Object.entries(channel.model_mapping ?? {}).map(([public_model, upstream_model]) => ({ public_model, upstream_model })) } : defaults
}

export function ChannelEditor({ channel, onClose, onSaved }: { channel?: Channel; onClose: () => void; onSaved: () => Promise<void> }) {
  const [discard, setDiscard] = useState(false)
  const [source, setSource] = useState<Source>('official')
  const form = useForm<ChannelValues>({ resolver: zodResolver(channel ? channelEditSchema : channelSchema), defaultValues: initialValues(channel) })
  const mappings = useFieldArray({ control: form.control, name: 'mappings' })
  const values = form.watch()
  const providers = useProviders().data ?? []
  const preset = providers.find(item => item.id === values.provider)
  // A preset fills protocol and base URL; the URL stays editable.
  const choosePreset = (id: string, protocol = form.getValues('protocol')) => {
    const options = { shouldDirty: true, shouldValidate: true }
    form.setValue('provider', id, options)
    const next = providers.find(item => item.id === id)
    if (!next?.endpoints.length) return
    const nextProtocol = next.endpoints.some(item => item.protocol === protocol) ? protocol : next.endpoints[0].protocol
    const endpoint = next.endpoints.find(item => item.protocol === nextProtocol)!
    form.setValue('protocol', nextProtocol, options)
    form.setValue('base_url', endpoint.base_url, options)
    if (!form.getValues('name').trim()) form.setValue('name', next.label, options)
  }
  const rules = modelNames(values.models)
  const mapping = Object.fromEntries(values.mappings.map(item => [item.public_model.trim(), item.upstream_model.trim()]))
  const passthrough = values.model_mode === 'passthrough'
  const testModelRequired = passthrough || !rules.some(isConcreteModel)
  const firstModel = !passthrough ? rules.find(isConcreteModel) : undefined
  const testModel = values.test_model.trim() || (firstModel ? mapping[firstModel] || firstModel : '')
  const draft = connection(values, channel?.id)
  const connectionSignature = JSON.stringify(draft)
  const checkSignature = JSON.stringify({ connection: draft, model: testModel, rules, mode: values.model_mode, mappings: values.mappings })
  // Fetched models join the rules, keeping rules already entered; in passthrough they are only test model options.
  const previewModels = useMutation({ mutationFn: (draft: Connection) => api<{ models: string[] }>('/channels/preview-models', jsonBody(draft)), onSuccess: ({ models }) => {
    if (form.getValues('model_mode') === 'passthrough') return
    const current = modelNames(form.getValues('models'))
    if (models.some(model => !current.includes(model))) form.setValue('models', [...new Set([...current, ...models])].sort().join('\n'), { shouldDirty: true, shouldValidate: true })
  } })
  const previewCheck = useMutation({ mutationFn: ({ draft, model }: { draft: Connection; model: string; signature: string }) => api<CheckResult>('/channels/preview-check', jsonBody({ ...draft, model })) })
  const { reset: resetModels } = previewModels
  const { reset: resetCheck } = previewCheck
  const [clock, setClock] = useState(0)
  const checkExpiresAt = previewCheck.data ? previewCheck.submittedAt + CHECK_TTL_MS : 0
  useEffect(() => {
    if (!checkExpiresAt) return
    const timer = setTimeout(() => setClock(Date.now()), Math.max(0, checkExpiresAt - Date.now()))
    return () => clearTimeout(timer)
  }, [checkExpiresAt])
  const checkExpired = !!checkExpiresAt && clock >= checkExpiresAt
  useEffect(() => { resetModels() }, [connectionSignature, resetModels])
  useEffect(() => { resetCheck(); form.clearErrors('root') }, [checkSignature, resetCheck, form])
  const modelsAreCurrent = JSON.stringify(previewModels.variables) === connectionSignature
  const checkMatches = previewCheck.variables?.signature === checkSignature
  const checkIsCurrent = checkMatches && !checkExpired
  const discovered = modelsAreCurrent && previewModels.data ? previewModels.data.models : connectionSignature === JSON.stringify(connection(initialValues(channel), channel?.id)) ? channel?.discovered_models ?? [] : []
  const suggestions = useModelSuggestions(discovered, values.protocol)
  const save = useMutation({
    mutationFn: ({ values, receipt }: { values: ChannelValues; receipt?: string | null }) => {
      const payload = { name: values.name, base_url: values.base_url, priority: Number(values.priority), models: modelNames(values.models), model_mode: values.model_mode, test_model: values.test_model || null, ...(values.secret ? { secret: values.secret } : {}), ...(receipt ? { check_receipt: receipt } : {}) }
      return channel
        ? api(`/channels/${channel.id}`, jsonBody({ ...payload, model_mapping: Object.fromEntries(values.mappings.map(item => [item.public_model, item.upstream_model])) }, 'PATCH'))
        : api('/channels', jsonBody({ ...payload, protocol: values.protocol, provider: values.provider === 'custom' ? null : values.provider }))
    },
    onSuccess: async () => { await onSaved(); onClose() },
  })
  const busy = save.isPending || previewModels.isPending || previewCheck.isPending
  const dirty = form.formState.isDirty
  const requestClose = () => { if (!busy) { if (dirty) setDiscard(true); else onClose() } }
  // Official channels must name their vendor before anything reaches the upstream.
  const vendorChosen = () => {
    if (channel || source !== 'official' || preset) return true
    form.setError('provider', { message: '请选择厂商；其他服务请选「第三方 API Key」' })
    return false
  }
  const fetchModels = async () => {
    if (busy || !vendorChosen() || !await form.trigger([...channelConnectionFields])) return
    previewModels.mutate(connection(form.getValues(), channel?.id))
  }
  const testNow = async () => {
    if (busy || !vendorChosen() || !await form.trigger([...channelConnectionFields, 'test_model', 'mappings'])) return
    if (!testModel) { form.setError('test_model', { message: '请选择或填写具体测试模型' }); return }
    form.clearErrors('root')
    previewCheck.mutate({ draft, model: testModel, signature: checkSignature })
  }
  const errors = form.formState.errors
  const fetchModelsAction = <Button type="button" variant="ghost" size="sm" className="h-7 px-2 text-xs" onClick={fetchModels}><RefreshCw aria-hidden="true" className="size-3.5" />{previewModels.isPending ? '获取中…' : '获取上游模型'}</Button>
  const fetchedModelsStatus = modelsAreCurrent && previewModels.data && <p role="status" className="text-xs text-muted-foreground">已获取 {previewModels.data.models.length} 个模型{passthrough ? '，可选作测试模型' : '，已加入模型规则'}。</p>
  const chooseSource = (next: Source) => {
    if (next === 'custom' && preset) {
      // Drop the preset's address so a relay is not created against the vendor by mistake.
      const options = { shouldDirty: true, shouldValidate: true }
      if (preset.endpoints.some(item => item.base_url === form.getValues('base_url'))) form.setValue('base_url', '', options)
      form.setValue('provider', 'custom', options)
    }
    form.clearErrors('provider')
    setSource(next)
  }
  return <>
    <FormSheet open onOpenChange={open => { if (!open) requestClose() }} title={channel ? '编辑渠道' : '添加上游渠道'} description={!channel && source === 'vair_key' ? '粘贴其他 VAIR 帐号提供给你的 Key，把它添加为渠道。' : undefined}>
      {!channel && <div className="mb-4"><Field label="渠道来源">{id => <SegmentedChoice id={id} aria-label="渠道来源" value={source} onChange={value => chooseSource(value as Source)} disabled={busy} options={sourceOptions} />}</Field></div>}
      {source === 'vair_key' ? <VairKeyChannelForm onCancel={onClose} onCreated={async () => { await onSaved(); onClose() }} /> : <form className="grid gap-4" onSubmit={form.handleSubmit(values => {
        if (busy || !vendorChosen()) return
        if (!channel && (!checkIsCurrent || !previewCheck.data?.healthy)) { form.setError('root', { message: '请先测试连接，通过后再创建渠道' }); return }
        save.mutate({ values, receipt: checkIsCurrent && previewCheck.data?.healthy ? previewCheck.data.check_receipt : undefined })
      })}>
        <fieldset disabled={busy} className="grid min-w-0 gap-4">
          <Field label="渠道名称" error={errors.name?.message}>{id => <Input id={id} autoComplete="off" placeholder="OpenAI 主渠道" {...form.register('name')} />}</Field>
          {!channel && source === 'official' && <Field label="厂商" error={errors.provider?.message} help="选择厂商后自动填写协议和国内站 Base URL，仍可修改；国际站 Key 请改为国际站地址。">{id => <Choice id={id} value={preset ? values.provider : ''} onChange={choosePreset} disabled={busy} options={providers.map(item => ({ value: item.id, label: item.label }))} />}</Field>}
          {!channel && <Field label="协议" error={errors.protocol?.message} hint={preset ? 'Claude Code 选 Anthropic 兼容；Codex 等 OpenAI 客户端选 OpenAI 兼容。' : undefined}>{id => <Controller name="protocol" control={form.control} render={({ field }) => <Choice id={id} value={field.value} onChange={value => preset ? choosePreset(preset.id, value as Protocol) : field.onChange(value)} disabled={busy} options={preset ? protocolOptions.filter(option => preset.endpoints.some(item => item.protocol === option.value)) : protocolOptions} />} />}</Field>}
          <Field label="Base URL" error={errors.base_url?.message} hint={channel?.provider ? `创建自厂商预设：${providers.find(item => item.id === channel.provider)?.label ?? channel.provider}` : undefined}>{id => <Input id={id} autoComplete="url" placeholder="https://api.example.com/v1" {...form.register('base_url')} />}</Field>
          <Field label="上游 API Key" error={errors.secret?.message} help={channel ? '留空使用已保存的 Key；填写新值后可直接测试。' : undefined}>{id => <Input id={id} type="password" autoComplete="new-password" spellCheck={false} maxLength={4096} {...form.register('secret')} />}</Field>
          <Field label="渠道优先级" error={errors.priority?.message} help="数值越小越优先。">{id => <Input id={id} type="number" min="0" {...form.register('priority')} />}</Field>
          <Field label="模型规则" error={passthrough ? undefined : errors.models?.message} help={passthrough ? undefined : '支持具体名称或前缀，如 gpt-*。'} action={<div className="flex items-center gap-3">{!passthrough && fetchModelsAction}<label className="flex items-center gap-2 text-sm"><Checkbox checked={passthrough} disabled={busy} onCheckedChange={checked => form.setValue('model_mode', checked === true ? 'passthrough' : 'allowlist', { shouldDirty: true, shouldValidate: true })} />全部转发</label></div>}>{id => passthrough ? <p id={id} className="rounded-md border border-dashed border-border px-3 py-2.5 text-sm text-muted-foreground">请求的模型原样转发到此渠道，不按规则过滤，是否支持由上游决定。</p> : <ModelTagInput id={id} values={rules} suggestions={suggestions} onChange={models => form.setValue('models', models.join('\n'), { shouldDirty: true, shouldValidate: true })} disabled={busy} placeholder="输入模型名或前缀规则" />}</Field>
          {!passthrough && fetchedModelsStatus}
          <Field label={testModelRequired ? '测试模型（必填）' : '测试模型（选填）'} error={errors.test_model?.message} help={testModelRequired ? '全部转发或仅前缀规则时，需要指定具体模型进行测试。' : '留空使用按名称排序后的第一个具体模型；配置了映射时使用其上游名称。'} action={passthrough ? fetchModelsAction : undefined}>{id => <Controller name="test_model" control={form.control} render={({ field }) => <ModelSelect id={id} name="test_model" value={field.value} onChange={field.onChange} models={rules.map(model => mapping[model] || model).concat(suggestions.map(model => model.value), values.mappings.map(item => item.upstream_model))} invalid={!!errors.test_model} disabled={busy} />} />}</Field>
          {passthrough && fetchedModelsStatus}
          {channel && <div className="grid gap-3 rounded-lg border border-border p-4">
            <div className="flex items-center justify-between gap-3"><p className="text-sm font-medium">模型映射</p><Button type="button" variant="outline" size="sm" onClick={() => mappings.append({ public_model: '', upstream_model: '' })}><Plus aria-hidden="true" className="size-4" />添加映射</Button></div>
            {mappings.fields.map((mapping, index) => <div key={mapping.id} className="grid gap-1">
              <div className="grid grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)_auto] items-center gap-2">
                <Input aria-label="对外模型名" spellCheck={false} placeholder="对外模型名" aria-invalid={!!errors.mappings?.[index]?.public_model} {...form.register(`mappings.${index}.public_model`)} />
                <ArrowRight aria-hidden="true" className="size-4 text-muted-foreground" />
                <Input aria-label="上游模型名" spellCheck={false} placeholder="上游模型名" aria-invalid={!!errors.mappings?.[index]?.upstream_model} {...form.register(`mappings.${index}.upstream_model`)} />
                <Button type="button" variant="ghost" size="icon" aria-label="删除模型映射" onClick={() => mappings.remove(index)}><Trash2 aria-hidden="true" className="size-4 text-destructive" /></Button>
              </div>
              {(errors.mappings?.[index]?.public_model || errors.mappings?.[index]?.upstream_model) && <p role="alert" className="text-xs text-destructive">{errors.mappings?.[index]?.public_model?.message ?? errors.mappings?.[index]?.upstream_model?.message}</p>}
            </div>)}
            {errors.mappings?.root?.message && <p role="alert" className="text-xs text-destructive">{errors.mappings.root.message}</p>}
          </div>}
        </fieldset>
        <ErrorBox error={save.error ?? (modelsAreCurrent ? previewModels.error : null) ?? (checkIsCurrent ? previewCheck.error : null)} />
        {errors.root?.message && <p role="alert" className="text-sm text-destructive">{errors.root.message}</p>}
        {checkMatches && checkExpired && <p role="status" className="text-sm text-muted-foreground">检查结果已超过有效期，请重新{channel ? '检查渠道' : '测试连接'}。</p>}
        {checkIsCurrent && previewCheck.data && <p role="status" className={`flex items-center gap-1.5 text-sm ${previewCheck.data.rate_limited ? 'text-warning' : previewCheck.data.healthy ? 'text-success' : 'text-destructive'}`}>{previewCheck.data.rate_limited ? <><CheckCircle2 aria-hidden="true" className="size-4" />地址和 Key 有效，但上游当前限流（HTTP 429），可以创建渠道</> : previewCheck.data.healthy ? <><CheckCircle2 aria-hidden="true" className="size-4" />连接测试通过</> : `测试未通过：${previewCheck.data.message}`}</p>}
        <div className="sticky bottom-0 z-10 flex flex-wrap items-center justify-between gap-3 border-t border-border bg-background py-4">
          <Button type="button" variant="outline" disabled={busy} onClick={testNow}>{previewCheck.isPending ? <LoaderCircle aria-hidden="true" className="size-4 animate-spin" /> : <ShieldCheck aria-hidden="true" className="size-4" />}{previewCheck.isPending ? '测试中…' : channel ? '检查渠道' : '测试连接'}</Button>
          <div className="flex gap-2"><Button type="button" variant="outline" disabled={busy} onClick={requestClose}>取消</Button><Button type="submit" disabled={busy}>{save.isPending ? '保存中…' : channel ? '保存设置' : '创建渠道'}</Button></div>
        </div>
      </form>}
    </FormSheet>
    <Confirm open={discard} onOpenChange={setDiscard} title="放弃未保存的修改？" description="当前修改尚未保存。" pending={false} onConfirm={onClose} />
  </>
}
