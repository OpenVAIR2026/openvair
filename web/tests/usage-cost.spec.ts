import { expect, test, type Page } from '@playwright/test'
import type { UsageSummary } from '../src/lib/api'

const coverage = { cache_reported_requests: 0, cache_unreported_requests: 0, cache_unknown_requests: 0, cache_reported_input_tokens: 0, cache_reported_tokens: 0 }
const channel = (name: string): UsageSummary['channels'][number] => ({ ...coverage, channel_id: name, channel_name: name, requests: 6, successful: 5, client_errors: 0, server_errors: 1, input_tokens: 100, output_tokens: 20, cached_tokens: 0, avg_ttfb_ms: 100, avg_duration_ms: 300, estimated_cost_usd: null })

async function fixtures(page: Page, trend: UsageSummary['trend'] = []) {
  const rows: UsageSummary['channels'] = [
    { ...channel('已计价渠道'), estimated_cost_usd: 2.5 },
    { ...channel('无费用渠道') },
    { ...channel('零价模型'), estimated_cost_usd: 0 },
  ]
  await page.route('**/api/**', async route => {
    const url = new URL(route.request().url())
    const send = (body: unknown) => route.fulfill({ contentType: 'application/json', body: JSON.stringify(body) })
    if (url.pathname === '/api/me') return send({ user_id: 'fixture', account_id: 'fixture', email: 'fixture@example.invalid', is_super_admin: false })
    if (url.pathname === '/api/account') return send({ id: 'fixture', name: '费用展示验证' })
    if (url.pathname === '/api/channels') return send([])
    if (url.pathname === '/api/usage/summary') return send({ since: new Date().toISOString(), bucket_seconds: 3600, totals: { ...coverage, requests: 18, successful: 15, input_tokens: 300, output_tokens: 60, cached_tokens: 0, active_models: 3, avg_duration_ms: 300, avg_ttfb_ms: 100 }, trend, models: [], channels: rows, model_options: [], key_options: [] } satisfies UsageSummary)
    if (url.pathname === '/api/usage/events') return send({ items: [], total: 0, page: 1, page_size: 10 })
    return route.fulfill({ status: 404, contentType: 'application/json', body: '{}' })
  })
  return rows
}

test('channel costs show priced amounts without unknown-cost hints', async ({ page }, testInfo) => {
  await fixtures(page)
  await page.goto('/usage')
  const row = page.getByRole('row').filter({ hasText: '已计价渠道' })
  await expect(row).toContainText('$2.5000')
  await expect(page.getByRole('row').filter({ hasText: '无费用渠道' })).toContainText('$0.0000')
  await expect(page.getByRole('row').filter({ hasText: '零价模型' })).toContainText('$0.0000')
  await expect(page.getByRole('button', { name: '查看未计价模型' })).toHaveCount(0)
  await expect(page.getByText(/未计价|费用未知|未计入/)).toHaveCount(0)
  await page.screenshot({ path: testInfo.outputPath('channel-cost-desktop.png'), fullPage: true, animations: 'disabled' })
})

test('cost trend shows priced spend in one color', async ({ page }) => {
  const start = Date.now() - 3 * 3_600_000
  await fixtures(page, [
    { start: new Date(start).toISOString(), requests: 2, tokens: 10, estimated_cost_usd: 1 },
    { start: new Date(start + 3_600_000).toISOString(), requests: 3, tokens: 10, estimated_cost_usd: 2 },
  ] as UsageSummary['trend'])
  await page.goto('/usage')
  await page.getByRole('combobox', { name: '图表统计方式', exact: true }).click()
  await page.getByRole('option', { name: '按费用', exact: true }).click()
  await expect(page.getByText('琥珀色', { exact: false })).toHaveCount(0)
  const bars = page.locator('.usage-chart .recharts-bar-rectangle path')
  await expect(bars).toHaveCount(2)
  for (const index of [0, 1]) await expect(bars.nth(index)).toHaveAttribute('fill', '#78d9d0')
})
