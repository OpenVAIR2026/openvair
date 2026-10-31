import { z } from 'zod'
import { isConcreteModel, isModelRule, splitModelTokens } from './model-rules'
const name = z.string().trim().min(1, '不能为空').max(120, '最长 120 个字符')
const positive = z.string().trim().refine(v => /^\d+$/.test(v) && Number.isSafeInteger(Number(v)) && Number(v) > 0, '请输入正整数')
const optionalPositive = z.string().trim().refine(v => !v || /^\d+$/.test(v) && Number.isSafeInteger(Number(v)) && Number(v) > 0, '请输入正整数或留空')
const priority = z.string().trim().refine(v => /^\d+$/.test(v) && Number(v) <= 2147483647, '请输入非负整数')
const channelShape = z.object({
  name,
  // 'custom' or a vendor preset id; presets only fill protocol and base URL.
  provider: z.string(),
  protocol: z.enum(['openai', 'anthropic', 'gemini']),
  base_url: z.string().trim().url('请输入有效 URL').refine(v => /^https?:\/\//.test(v), '只支持 HTTP/HTTPS'),
  secret: z.string().trim().max(4096, 'Key 过长'),
  models: z.string().refine(v => splitModelTokens(v).every(isModelRule), '请输入具体模型名或末尾 * 前缀'),
  model_mode: z.enum(['allowlist', 'passthrough']),
  test_model: z.string().trim().refine(v => !v || isConcreteModel(v), '测试模型必须是具体名称'),
  priority,
  concurrency_limit: z.string().trim().refine(v => !v || /^\d+$/.test(v) && Number(v) >= 1 && Number(v) <= 10000, '请输入 1–10000 的整数或留空'),
  mappings: z.array(z.object({
    public_model: z.string().trim().refine(isConcreteModel, '请输入具体的对外模型名'),
    upstream_model: z.string().trim().refine(isConcreteModel, '请输入具体的上游模型名'),
  })).max(1000, '最多 1000 条映射'),
})
function validateChannel(value: z.infer<typeof channelShape>, ctx: z.RefinementCtx) {
  const rules = splitModelTokens(value.models)
  if (value.model_mode === 'allowlist' && !rules.length) ctx.addIssue({ code: 'custom', path: ['models'], message: '至少添加一条模型规则' })
  if ((value.model_mode === 'passthrough' || !rules.some(isConcreteModel)) && !value.test_model) ctx.addIssue({ code: 'custom', path: ['test_model'], message: '全部转发或仅前缀规则时必须填写具体测试模型' })
  const seen = new Set<string>()
  value.mappings.forEach((mapping, index) => {
    if (seen.has(mapping.public_model)) ctx.addIssue({ code: 'custom', path: ['mappings', index, 'public_model'], message: '同一渠道的对外模型名不能重复' })
    seen.add(mapping.public_model)
  })
}
export const channelSchema = channelShape.extend({ secret: channelShape.shape.secret.min(1, '请输入 API Key') }).superRefine(validateChannel)
export const channelEditSchema = channelShape.superRefine(validateChannel)
export const channelConnectionFields = ['protocol', 'base_url', 'secret'] as const
const optionalPositiveDecimal = z.string().trim().refine(v => !v || (/^\d*\.?\d+$/.test(v) && Number(v) > 0), '请输入正数或留空')
// Limits that are added on demand; an absent limit means unlimited.
export const keyLimitFields = ['token_limit_daily', 'cost_limit_daily_usd', 'cost_limit_total_usd'] as const
/** Channel and model scope, shared by keys and key groups. */
const scopeShape = { model_mode: z.enum(['all', 'allowlist']), allowed_models: z.string().refine(v => splitModelTokens(v).every(isModelRule), '模型规则只能在末尾包含一个 *'), channel_mode: z.enum(['all', 'selected']), channel_ids: z.array(z.string()) }
function refineScope(value: { model_mode: string; allowed_models: string; channel_mode: string; channel_ids: string[] }, ctx: z.RefinementCtx) {
  if (value.model_mode === 'allowlist' && !splitModelTokens(value.allowed_models).length) ctx.addIssue({ code: 'custom', path: ['allowed_models'], message: '至少添加一条规则，或显式选择允许全部' })
  if (value.channel_mode === 'selected' && !value.channel_ids.length) ctx.addIssue({ code: 'custom', path: ['channel_ids'], message: '至少选择一个渠道，或勾选允许全部渠道' })
}
/** `group_id` is '' for no group or NEW_GROUP to create one named `new_group`. */
export const NEW_GROUP = 'new'
export const keySchema = z.object({ name, holder: z.string().trim().max(120, '最长 120 个字符'), ...scopeShape, group_id: z.string(), new_group: z.string().trim().max(120, '最长 120 个字符'), requests_per_minute: optionalPositive.refine(v => !v || Number(v) <= 2147483647, '请求数过大'), token_limit_daily: optionalPositiveDecimal, cost_limit_daily_usd: optionalPositiveDecimal, cost_limit_total_usd: optionalPositiveDecimal, active_limits: z.array(z.enum(keyLimitFields)), concurrency_limit: positive.refine(v => Number(v) <= 100, '并发上限为 100') }).superRefine((value, ctx) => {
  refineScope(value, ctx)
  if (value.group_id === NEW_GROUP && !value.new_group) ctx.addIssue({ code: 'custom', path: ['new_group'], message: '请输入分组名称' })
  for (const field of value.active_limits) if (!value[field]) ctx.addIssue({ code: 'custom', path: [field], message: '请输入上限，或移除此限制' })
})
export const keyGroupSchema = z.object({ name, ...scopeShape }).superRefine(refineScope)
export type ChannelValues = z.infer<typeof channelSchema>
export type KeyValues = z.infer<typeof keySchema>
export type KeyGroupValues = z.infer<typeof keyGroupSchema>
