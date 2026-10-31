import { expect, test } from '@playwright/test'
import { cacheMetric, successTone, usageAxisTick, usageCost } from '../src/lib/usage-metrics'

test('trend axes abbreviate large counts and retain small USD charges', () => {
  expect(usageAxisTick(0)).toBe('0')
  expect(usageAxisTick(1200)).toBe('1.2K')
  expect(usageAxisTick(123_456_789)).toBe('123.5M')
  expect(usageAxisTick(2_000_000_000)).toBe('2B')
  expect(usageAxisTick(12000, true)).toBe('$12K')
  const tinyTicks = [0, 0.000001, 0.0000015, 0.000002].map(value => usageAxisTick(value, true))
  expect(new Set(tinyTicks).size).toBe(4)
  expect(usageAxisTick(0.0000015, true)).toBe('$0.0000015')
  expect(usageAxisTick(0.0000002, true)).toBe('$2.0e-7')
  expect(usageCost(0.001234)).toBe('$0.001234')
  expect(usageCost(0.0000002)).toBe('< $0.000001')
  expect(usageCost(0)).toBe('$0.0000')
})

const reported = {
  input_tokens: 100,
  cache_reported_requests: 1,
  cache_unreported_requests: 0,
  cache_unknown_requests: 0,
  cache_reported_input_tokens: 100,
  cache_reported_tokens: 0,
}

test('success bands include 95% and 80% without rounding before comparison', () => {
  expect(successTone(95, 100)).toBe('success')
  expect(successTone(9499, 10000)).toBe('warning')
  expect(successTone(80, 100)).toBe('warning')
  expect(successTone(7999, 10000)).toBe('danger')
  expect(successTone(0, 0)).toBe('neutral')
  expect(successTone(0, 1)).toBe('danger')
})

test('reported zero, missing usage, legacy unknown and no input stay distinct', () => {
  expect(cacheMetric(reported)).toMatchObject({ value: '0.0%', rate: 0, level: 'none' })
  expect(cacheMetric({ ...reported, cache_reported_requests: 0, cache_reported_input_tokens: 0, cache_unreported_requests: 1 })).toMatchObject({ value: '未上报', rate: null })
  expect(cacheMetric({ ...reported, cache_reported_requests: 0, cache_reported_input_tokens: 0, cache_unknown_requests: 1 })).toMatchObject({ value: '未知', detail: '历史数据', rate: null })
  expect(cacheMetric({ ...reported, input_tokens: 0 })).toMatchObject({ value: '—', detail: '无输入用量', rate: null })
})

test('partial reporting uses only matching reported input as its denominator', () => {
  const result = cacheMetric({ ...reported, input_tokens: 1000, cache_reported_tokens: 50, cache_unreported_requests: 8, cache_unknown_requests: 1 })
  expect(result).toMatchObject({ rate: 50, value: '50.0%' })
  expect(result.title).toContain('8 次未上报')
  expect(result.title).toContain('1 次上报情况未知')
})

test('cache emphasis changes at the agreed boundaries of 30% and 70%', () => {
  for (const [tokens, level] of [[0, 'none'], [1, 'low'], [29, 'low'], [30, 'medium'], [69, 'medium'], [70, 'high'], [100, 'high']] as const) {
    expect(cacheMetric({ ...reported, cache_reported_tokens: tokens }).level).toBe(level)
  }
})
