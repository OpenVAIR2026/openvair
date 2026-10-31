import { expect, test, type Page } from '@playwright/test'
import type { UsageSummary } from '../src/lib/api'

const coverage = { cache_reported_requests: 0, cache_unreported_requests: 0, cache_unknown_requests: 0, cache_reported_input_tokens: 0, cache_reported_tokens: 0 }
const week = 7 * 86_400

async function fixtures(page: Page) {
  const queries: URLSearchParams[] = []
  await page.route('**/api/**', async route => {
    const url = new URL(route.request().url())
    const send = (body: unknown) => route.fulfill({ contentType: 'application/json', body: JSON.stringify(body) })
    if (url.pathname === '/api/me') return send({ user_id: 'fixture', account_id: 'fixture', email: 'fixture@example.invalid', is_super_admin: false })
    if (url.pathname === '/api/account') return send({ id: 'fixture', name: '全部时间验证' })
    if (url.pathname === '/api/channels') return send([])
    if (url.pathname === '/api/usage/summary') {
      queries.push(url.searchParams)
      const all = url.searchParams.get('range') === 'all'
      const step = all ? week : 3600
      const start = new Date(2026, 6, 6).getTime()
      const trend = Array.from({ length: all ? 14 : 24 }, (_, index) => ({ start: new Date(start + index * step * 1000).toISOString(), requests: index % 3 + 1, tokens: 100, estimated_cost_usd: 0.1 }))
      return send({ since: trend[0].start, bucket_seconds: step, totals: { ...coverage, requests: 30, successful: 30, input_tokens: 1000, output_tokens: 200, cached_tokens: 0, active_models: 1, avg_duration_ms: 300, avg_ttfb_ms: 100 }, trend, models: [], channels: [], model_options: [], key_options: [] } satisfies UsageSummary)
    }
    if (url.pathname === '/api/usage/events') return send({ items: [], total: 0, page: 1, page_size: 10 })
    return route.fulfill({ status: 404, contentType: 'application/json', body: '{}' })
  })
  return queries
}

test('usage analysis offers all time with wider buckets, while the overview keeps recent windows', async ({ page }, testInfo) => {
  const queries = await fixtures(page)
  await page.goto('/usage')
  const range = page.getByRole('combobox', { name: '数据范围' })
  await range.click()
  await page.getByRole('option', { name: '全部时间', exact: true }).click()
  await expect(page).toHaveURL(/range=all/)
  await expect.poll(() => queries.at(-1)?.get('range')).toBe('all')
  await expect(page.getByText('按周查看', { exact: false })).toBeVisible()
  await expect(range).toHaveText('全部时间')
  await page.screenshot({ path: testInfo.outputPath('usage-all-time.png'), fullPage: true })
  await page.reload()
  await expect(page.getByRole('combobox', { name: '数据范围' })).toHaveText('全部时间')
  await page.goto('/overview?range=all')
  await expect(page.getByRole('heading', { name: '运行概览' })).toBeVisible()
  // The overview ignores `all` and falls back to its default window.
  await expect.poll(() => queries.at(-1)?.get('range')).toBe('24h')
  await page.getByRole('combobox', { name: '统计范围' }).click()
  await expect(page.getByRole('option', { name: '全部时间', exact: true })).toHaveCount(0)
})
