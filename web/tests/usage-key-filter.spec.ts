import { expect, test, type Page } from '@playwright/test'
import type { UsageEvent, UsageSummary } from '../src/lib/api'

const coverage = { cache_reported_requests: 0, cache_unreported_requests: 0, cache_unknown_requests: 0, cache_reported_input_tokens: 0, cache_reported_tokens: 0 }
const event: UsageEvent = { id: 'event', channel_id: 'channel', channel_name: '主渠道', virtual_key_id: 'retired', key_name: '旧应用', key_revoked: true, model: 'fixture-model', protocol: 'openai', status: 200, ttfb_ms: 100, duration_ms: 300, input_tokens: 100, output_tokens: 20, cached_tokens: null, created_at: new Date().toISOString(), upstream_model: null, price_snapshot: null, cost_usd: 0.25, pricing_status: 'priced' }

async function fixtures(page: Page) {
  const queries: URLSearchParams[] = []
  await page.route('**/api/**', async route => {
    const url = new URL(route.request().url())
    const send = (body: unknown) => route.fulfill({ contentType: 'application/json', body: JSON.stringify(body) })
    if (url.pathname === '/api/me') return send({ user_id: 'fixture', account_id: 'fixture', email: 'fixture@example.invalid', role: 'owner', is_super_admin: false })
    if (url.pathname === '/api/account') return send({ id: 'fixture', name: 'Key 筛选验证' })
    if (url.pathname === '/api/channels') return send([])
    if (url.pathname === '/api/usage/summary') {
      queries.push(url.searchParams)
      return send({ since: new Date().toISOString(), bucket_seconds: 3600, totals: { ...coverage, requests: 1, successful: 1, input_tokens: 100, output_tokens: 20, cached_tokens: 0, active_models: 1, avg_duration_ms: 300, avg_ttfb_ms: 100 }, trend: [], models: [], channels: [], model_options: [], key_options: [{ id: 'production', name: '生产应用', revoked: false }, { id: 'retired', name: '旧应用', revoked: true }] } satisfies UsageSummary)
    }
    if (url.pathname === '/api/usage/events') return send({ items: [event], total: 1, page: 1, page_size: 10 })
    return route.fulfill({ status: 404, contentType: 'application/json', body: '{}' })
  })
  return queries
}

test('a key from the URL filters usage and can be cleared from the header', async ({ page }, testInfo) => {
  const queries = await fixtures(page)
  await page.goto('/usage?key=retired')
  const header = page.getByRole('combobox', { name: 'API Key 筛选' }).first()
  await expect(header).toHaveText('旧应用（已撤销）')
  expect(queries.at(-1)?.get('key_id')).toBe('retired')
  await expect(page.getByRole('row').filter({ hasText: 'fixture-model' })).toContainText('旧应用（已撤销）')
  await page.screenshot({ path: testInfo.outputPath('usage-key-filter-desktop.png'), fullPage: true, animations: 'disabled' })
  await header.click()
  await page.getByRole('option', { name: '全部 Key', exact: true }).click()
  await expect(page).toHaveURL(/\/usage$/)
  await expect.poll(() => queries.at(-1)?.has('key_id')).toBe(false)
})

test('picking a key in the detail filters updates the query', async ({ page }) => {
  const queries = await fixtures(page)
  await page.goto('/usage')
  await page.getByRole('combobox', { name: 'API Key 筛选' }).last().click()
  await page.getByRole('option', { name: '生产应用', exact: true }).click()
  await expect(page).toHaveURL(/key=production/)
  await expect.poll(() => queries.at(-1)?.get('key_id')).toBe('production')
})
