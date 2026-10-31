import type { CacheCoverage } from './api'

const axisNumber = new Intl.NumberFormat('en-US', { notation: 'compact', maximumFractionDigits: 1 })
const axisCost = new Intl.NumberFormat('en-US', { maximumSignificantDigits: 3 })

export function usageAxisTick(value: number, cost = false) {
  if (!cost) return axisNumber.format(value)
  // Keep tiny charges visible and large totals short enough for the axis gutter.
  if (value > 0 && value < 0.000001) return `$${value.toExponential(1)}`
  if (value >= 1000) return `$${axisNumber.format(value)}`
  return `$${axisCost.format(value)}`
}

export function usageCost(value: number) {
  if (value > 0 && value < 0.000001) return '< $0.000001'
  return `$${value.toFixed(value > 0 && value < 0.01 ? 6 : 4)}`
}

export function successTone(successful: number, requests: number): 'neutral' | 'success' | 'warning' | 'danger' {
  if (requests <= 0) return 'neutral'
  const rate = successful / requests
  return rate >= 0.95 ? 'success' : rate >= 0.8 ? 'warning' : 'danger'
}

export function cacheMetric(usage: CacheCoverage & { input_tokens: number }) {
  if (usage.input_tokens <= 0) return { rate: null, value: '—', detail: '无输入用量', title: '当前没有输入 Token', level: 'none' }
  // Missing coverage fields can occur while an older API is still serving during deployment.
  const reported = usage.cache_reported_requests ?? 0
  const unreported = usage.cache_unreported_requests ?? 0
  const unknown = usage.cache_unknown_requests ?? 0
  if (reported <= 0 || !(usage.cache_reported_input_tokens > 0)) {
    const missing = unknown === 0 && unreported > 0
    return { rate: null, value: missing ? '未上报' : '未知', detail: missing ? '' : '历史数据', title: missing ? '上游未上报缓存用量，不能视为 0%' : '历史记录无法区分缓存未命中与未上报', level: 'none' }
  }
  const rate = usage.cache_reported_tokens / usage.cache_reported_input_tokens * 100
  const partial = unreported > 0 || unknown > 0
  const title = `按 ${reported} 次已上报且含输入 Token 的请求计算；${unreported} 次未上报，${unknown} 次上报情况未知`
  return { rate, value: `${rate.toFixed(1)}%`, detail: '', title, level: rate >= 70 ? 'high' : rate >= 30 ? 'medium' : rate > 0 ? 'low' : 'none' }
}
