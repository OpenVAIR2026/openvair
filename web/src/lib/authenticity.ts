// Model authenticity checks: types and labels shared by the deep-check sheet
// and the 模型基准 admin page. See docs/model-authenticity-design.md.

export type ReasoningMode = 'off' | 'default'
export const reasoningModes: { value: ReasoningMode; label: string }[] = [
  { value: 'off', label: '关闭推理（默认）' },
  { value: 'default', label: '模型默认（不允许关闭推理的模型）' },
]

export type Verdict = 'match' | 'family_only' | 'suspected' | 'confirmed' | 'insufficient' | 'ambiguous' | 'unsupported'
export type Tone = 'neutral' | 'success' | 'warning' | 'danger'

export const verdicts: Record<Verdict, { label: string; tone: Tone; hint: string }> = {
  match: { label: '相符', tone: 'success', hint: '本次证据与申报模型的基准一致，并明显区别于其他候选模型。这不保证渠道从未偷换。' },
  family_only: { label: '仅确认厂商', tone: 'neutral', hint: '可以确认属于申报模型的厂商，但还不能与同家族型号区分。可增加样本数复查。' },
  suspected: { label: '疑似不符', tone: 'warning', hint: '数字分布更像其他模型，但证据还不足以确认。建议用更多样本复查。' },
  confirmed: { label: '确认不符', tone: 'danger', hint: '多类证据都指向其他模型，渠道很可能没有提供申报的模型。' },
  insufficient: { label: '证据不足', tone: 'neutral', hint: '有效样本太少或证据不一致，本次不下结论。' },
  ambiguous: { label: '模棱两可', tone: 'warning', hint: '数字分布与 Tokenizer 计数指向不同结论，可能存在协议转换或混合路由。' },
  unsupported: { label: '暂不支持', tone: 'neutral', hint: '申报模型还没有启用的基准。' },
}

export const verdictOf = (value: string | null | undefined) => value && value in verdicts ? verdicts[value as Verdict] : null

export interface Needed { compared?: boolean; mean: number | null; p10: number | null; evaluated_up_to?: number | null }

export interface DeepCheckModel {
  model: string
  family: string
  per_sample_cost_usd: number | null
  per_sample_tokens: { input: number; output: number }
  cross_family: Needed | null
  same_family: Needed | null
  has_siblings: boolean | null
  evaluated_at: string | null
}

export interface DeepCheckOptions {
  monitor_id: string
  model: string
  upstream_model: string
  claimed_default: string | null
  acknowledged: boolean
  tiers: number[]
  min_samples: number
  max_samples: number
  daily_sample_limit: number
  models: DeepCheckModel[]
}

export interface Comparison { model: string; family: string; same_family: boolean; mean_diff: number; z: number; token_consistent: boolean | null }

export interface DeepCheckEvidence {
  verdict: Verdict
  claimed: string
  claimed_family: string | null
  valid_samples: number
  best_model: string | null
  comparisons?: Comparison[]
  tokenizer: { observed_diff: number; claimed_diff: number | null; consistent: boolean | null; matching_families: string[] } | null
  reasons: string[]
  samples?: { completed: number; valid: number; invalid: Record<string, number>; target: number; token_probes: number; token_valid: number }
  usage?: { input_tokens: number; output_tokens: number }
  baseline?: { version_id: string; revision: number; pool_version: number }
  stop_reason?: string | null
  reasoning_mode?: ReasoningMode
  reasoning_param_rejected?: boolean
}

export interface DeepCheckJob {
  id: string
  /** deep: started by hand; scheduled: a sampling run judged with its window. */
  source: 'deep' | 'scheduled'
  status: 'queued' | 'running' | 'completed' | 'failed' | 'cancelled'
  claimed_model: string | null
  target_samples: number | null
  completed_samples: number
  valid_samples: number
  verdict: Verdict | null
  evidence: DeepCheckEvidence | null
  stopped_early: boolean
  error_code: string | null
  created_at: string
  finished_at: string | null
}

export interface BaselineEvaluation {
  model: string
  samples: number
  splits: number
  cross_family: Needed
  same_family: Needed
  pairs: { model: string; family: string; same_family: boolean; accuracy: Record<string, { mean: number; p10: number }> }[]
  skipped: string[]
  recommendation: string | null
}

export interface BaselineVersionSummary {
  id: string
  status: 'draft' | 'active' | 'retired'
  revision: number
  valid_samples: number
  pool_version: number
  created_at: string
  activated_at: string | null
  evaluation: BaselineEvaluation | null
  evaluated_revision: number | null
  evaluated_at: string | null
  progress: VersionProgress | null
}

/** Live batches of a version and its Tokenizer samples, counted per template against the target. */
export interface VersionProgress { batches: number; token_target: number; token_counted: number; token_missing: number }

export interface KindCount { total: number; valid: number; invalid: Record<string, number> }
export interface SampleCounts { fingerprint?: KindCount; token?: KindCount }

export interface BaselineModel {
  model: string
  label: string | null
  family: string
  openrouter_model: string
  provider_only: string
  target_samples: number
  token_samples: number
  reasoning: ReasoningMode
  active: BaselineVersionSummary | null
  draft: BaselineVersionSummary | null
  pending_batches: number
}

export interface OpenRouterLookup {
  candidates: { id: string; name: string | null; supports_reasoning: boolean }[]
  openrouter_model: string | null
  providers: { slug: string; name: string | null; quantization: string | null }[]
  family: string | null
  provider_only: string | null
}

export interface BaselineOverview { models: BaselineModel[]; pool_version: number | null; range_max: number }

export interface BaselineVersion extends BaselineVersionSummary {
  provider_only: string
  batches: { id: string; valid_count: number; sample_count: number; counts: SampleCounts; created_at: string; excluded_at: string | null }[]
}

export interface BaselineBatch {
  id: string
  model: string
  version_id: string | null
  status: 'pending' | 'merged' | 'rejected'
  manifest: { openrouter_model: string; provider_only: string; pool_version: number; tool_version: string; started_at: string; finished_at: string }
  validation: { samples: number; duplicates: number; valid: number; fingerprint_valid: number; token_valid: number; invalid: Record<string, number>; compatible_version_id: string | null }
  drift: { checked: boolean; jsd: number; p_value: number; flagged: boolean } | null
  preview: BaselineEvaluation | null
  sample_count: number
  valid_count: number
  counts: SampleCounts
  /** The version the batch joined, as it is now. */
  version: { revision: number; status: 'draft' | 'active' | 'retired'; valid_samples: number } | null
  compatible_revision: number | null
  target_samples: number | null
  created_at: string
  decided_at: string | null
  excluded_at: string | null
}

export interface ProbeTemplate { id: string; probe_kind: 'fingerprint' | 'token'; content: string; created_at: string; retired_at: string | null }
export interface PoolVersion { version: number; template_ids: string[]; published_at: string }

export interface BaselinePlan {
  id: string
  pool_version: number
  expires_at: string
  estimated_cost_usd: number
  items: { model: string; openrouter_model: string; provider_only: string; reasoning: ReasoningMode; version_id: string | null; fingerprint: number; token: Record<string, number> }[]
}

/** "16 个样本" / "未达到" from an evaluation figure, using the conservative 10th percentile. */
export function neededLabel(needed: Needed | null | undefined) {
  if (!needed) return '未评估'
  if (needed.compared === false) return '没有可比较的模型'
  const k = needed.p10 ?? needed.mean
  if (k != null) return `${k} 个样本`
  // Older evaluations lack evaluated_up_to; they tested up to 64.
  const limit = needed.evaluated_up_to === undefined ? 64 : needed.evaluated_up_to
  if (limit == null || limit < 128) return `${limit ?? 0} 个样本内未达到，更多样本需补采基准后才能评估`
  return '128 个样本内未达到'
}

/** What a sample count can reliably detect for a model, from its evaluation. */
export function tierCapability(samples: number, model: DeepCheckModel | undefined) {
  if (!model?.cross_family) return '该模型还没有评估结果'
  if (model.cross_family.compared === false && model.same_family?.compared === false) return '还没有其他启用的基准可比较'
  const cross = model.cross_family.p10 ?? model.cross_family.mean
  const sibling = model.same_family?.p10 ?? model.same_family?.mean
  if (model.has_siblings && sibling != null && samples >= sibling) return '可识别跨厂商替换和同家族降级'
  const evaluated = model.same_family?.evaluated_up_to
  if (model.has_siblings && sibling == null && evaluated != null && samples > evaluated) return '可识别跨厂商替换；同家族降级在该样本数下尚无法评估'
  if (cross != null && samples >= cross) return model.has_siblings ? '可识别跨厂商替换，同家族降级可能识别不出' : '可识别跨厂商替换'
  return '样本较少，可能只能得出"证据不足"'
}

export const invalidReasons: Record<string, string> = {
  empty: '空回答', no_numbers: '没有数字', out_of_range: '越界过多', too_few: '数量过少', reasoning: '上游开启了推理',
  provider_mismatch: '供应商不符', no_provider: '未返回供应商', unknown_template: '模板不在提示词池', no_usage: '缺少用量', bad_n: '数量参数无效',
  quality_probe_missing_credential: '缺少渠道凭据',
}

/** Standard normal CDF (Abramowitz–Stegun 7.1.26), enough for a displayed percentage. */
function normalCdf(z: number) {
  const x = Math.abs(z) / Math.SQRT2
  const t = 1 / (1 + 0.3275911 * x)
  const erf = 1 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496735) * t + 0.254829592) * t * Math.exp(-x * x)
  return z >= 0 ? (1 + erf) / 2 : (1 - erf) / 2
}

/**
 * How sure the samples are about the verdict, from the decisive comparison:
 * the closest other model for a match, the favoured one for a mismatch.
 * One-sided, so it reflects these samples only, not the channel's whole history.
 */
export function confidenceOf(evidence: Pick<DeepCheckEvidence, 'verdict' | 'comparisons'> | null | undefined) {
  const comparisons = evidence?.comparisons ?? []
  if (!evidence || !comparisons.length) return null
  const percent = (p: number) => p >= 0.999 ? '> 99.9%' : `约 ${(Math.floor(p * 1000) / 10).toFixed(1)}%`
  const lowest = (list: Comparison[]) => list.reduce<Comparison | null>((min, c) => !min || c.z < min.z ? c : min, null)
  if (evidence.verdict === 'match' || evidence.verdict === 'family_only') {
    const closest = lowest(evidence.verdict === 'family_only' ? comparisons.filter(c => !c.same_family) : comparisons)
    return closest ? { value: percent(normalCdf(closest.z)), detail: `与最接近的候选模型 ${closest.model} 相比，样本更符合申报模型（z = ${closest.z.toFixed(1)}）` } : null
  }
  if (evidence.verdict === 'suspected' || evidence.verdict === 'confirmed') {
    const favoured = lowest(comparisons)
    return favoured && favoured.z < 0 ? { value: percent(normalCdf(-favoured.z)), detail: `样本更接近 ${favoured.model}（z = ${favoured.z.toFixed(1)}）` } : null
  }
  return null
}
