import { expect, test, type Page } from '@playwright/test'
import type { UsageSummary } from '../src/lib/api'

const coverage = { cache_reported_requests: 0, cache_unreported_requests: 0, cache_unknown_requests: 0, cache_reported_input_tokens: 0, cache_reported_tokens: 0 }
const none = { p50_first_token_ms: null, p50_duration_ms: null, p50_output_tps: null, speed_samples: 0 }
const channel = (name: string, performance: Partial<UsageSummary['channels'][number]>): UsageSummary['channels'][number] => ({ ...coverage, ...none, channel_id: name, channel_name: name, requests: 40, successful: 40, client_errors: 0, server_errors: 0, input_tokens: 1000, output_tokens: 4000, cached_tokens: 0, avg_ttfb_ms: 900, avg_duration_ms: 5000, estimated_cost_usd: 0.1, ...performance })

async function fixtures(page: Page, trend: UsageSummary['trend']) {
  const channels = [
    channel('流式渠道', { p50_first_token_ms: 820, p50_duration_ms: 6400, p50_output_tps: 61.4, speed_samples: 12 }),
    channel('非流式渠道', { p50_duration_ms: 3000 }),
  ]
  await page.route('**/api/**', async route => {
    const url = new URL(route.request().url())
    const send = (body: unknown) => route.fulfill({ contentType: 'application/json', body: JSON.stringify(body) })
    if (url.pathname === '/api/me') return send({ user_id: 'fixture', account_id: 'fixture', email: 'fixture@example.invalid', is_super_admin: false })
    if (url.pathname === '/api/account') return send({ id: 'fixture', name: '输出性能验证' })
    if (url.pathname === '/api/channels') return send([])
    if (url.pathname === '/api/usage/summary') return send({ since: new Date().toISOString(), bucket_seconds: 3600, totals: { ...coverage, requests: 80, successful: 80, input_tokens: 2000, output_tokens: 8000, cached_tokens: 0, active_models: 1, avg_duration_ms: 5000, avg_ttfb_ms: 900 }, trend, models: [{ model: 'model-a', requests: 80, tokens: 10000, estimated_cost_usd: 0.2 }], channels, model_options: ['model-a'], key_options: [] })
    if (url.pathname === '/api/usage/events') return send({ items: [], total: 0, page: 1, page_size: 10 })
    return route.fulfill({ status: 404, contentType: 'application/json', body: '{}' })
  })
}

const bucket = (hour: number, performance: Partial<UsageSummary['trend'][number]>): UsageSummary['trend'][number] => ({ ...none, start: new Date(Date.now() - hour * 3600_000).toISOString(), requests: 10, tokens: 1000, estimated_cost_usd: 0.01, ...performance })

test('channels show median first output, duration and output speed', async ({ page }) => {
  await fixtures(page, [])
  await page.goto('/usage')
  const streamed = page.getByRole('row').filter({ hasText: '流式渠道' }).first()
  await expect(streamed).toContainText('820')
  await expect(streamed).toContainText('61.4 Token/秒')
  // Few streamed requests are flagged; without streams there is no speed.
  await expect(streamed).toContainText('12 个流式请求 · 样本较少')
  await expect(page.getByRole('row').filter({ hasText: '非流式渠道' })).toContainText('无流式请求')
})

test('the trend chart switches to output performance', async ({ page }) => {
  await fixtures(page, [bucket(2, { p50_first_token_ms: 700, p50_output_tps: 55 }), bucket(1, { p50_first_token_ms: 900, p50_output_tps: 48 })])
  await page.goto('/usage')
  await page.getByRole('combobox', { name: '图表统计方式' }).click()
  await page.getByRole('option', { name: '输出性能' }).click()
  await expect(page.getByRole('heading', { name: '输出性能' })).toBeVisible()
  await expect(page.getByText('流式请求的首字时间和输出速度（中位数）')).toBeVisible()
  await expect(page.locator('.usage-chart .recharts-line')).toHaveCount(2)
  // The model share keeps counting requests.
  await expect(page.getByText('80 次')).toBeVisible()
})

test('output performance explains when there are no streamed requests', async ({ page }) => {
  await fixtures(page, [bucket(1, {})])
  await page.goto('/usage')
  await page.getByRole('combobox', { name: '图表统计方式' }).click()
  await page.getByRole('option', { name: '输出性能' }).click()
  await expect(page.getByText('暂无输出性能数据')).toBeVisible()
})
