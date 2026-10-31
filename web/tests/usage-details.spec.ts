import { expect, test, type Page } from '@playwright/test'
import type { UsageRequestDetail } from '../src/lib/api'

const detail: UsageRequestDetail = {
  id: 'request-fixture', channel_id: 'backup', channel_name: '备用渠道', virtual_key_id: 'key-fixture', key_name: '开发 Key', key_revoked: false,
  model: 'fixture-model', upstream_model: 'upstream-model', protocol: 'openai', status: 200,
  ttfb_ms: 900, first_token_ms: 1000, duration_ms: 1600, stream: true, input_tokens: 126788, cached_tokens: 121216, output_tokens: 424,
  created_at: '2026-10-14T03:54:22Z', price_snapshot: null, cost_usd: 0.027506, pricing_status: 'priced', source: 'gateway',
  request_diagnostics: { queue_ms: 100, completion: 'success', attempts: [
    { channel_id: 'primary', channel_name: '主渠道', started_ms: 120, duration_ms: 500, headers_ms: 490, first_token_ms: null, status: 503, outcome: 'http_error' },
    { channel_id: 'backup', channel_name: '备用渠道', started_ms: 650, duration_ms: 950, headers_ms: 250, first_token_ms: 350, status: 200, outcome: 'success' },
  ] },
}

async function fixtures(page: Page, value = detail, options: { failOnce?: boolean; delay?: number } = {}) {
  let calls = 0
  const coverage = { cache_reported_requests: 1, cache_unreported_requests: 0, cache_unknown_requests: 0, cache_reported_input_tokens: 126788, cache_reported_tokens: 121216 }
  await page.route('**/api/**', async route => {
    const url = new URL(route.request().url())
    const send = (body: unknown) => route.fulfill({ contentType: 'application/json', body: JSON.stringify(body) })
    if (url.pathname === '/api/me') return send({ user_id: 'fixture', account_id: 'fixture', email: 'fixture@example.invalid', is_super_admin: false })
    if (url.pathname === '/api/account') return send({ id: 'fixture', name: '请求排查' })
    if (url.pathname === '/api/channels') return send([])
    if (url.pathname === '/api/usage/summary') return send({ since: detail.created_at, bucket_seconds: 3600, totals: { ...coverage, requests: 12, successful: 12, input_tokens: 126788, output_tokens: 424, cached_tokens: 121216, active_models: 1, avg_duration_ms: 1600, avg_ttfb_ms: 900 }, trend: [], models: [], channels: [], model_options: ['fixture-model'], key_options: [] })
    if (url.pathname === '/api/usage/events') return send({ items: [value], total: 12, page: Number(url.searchParams.get('page')), page_size: 10 })
    if (url.pathname === `/api/usage/events/${detail.id}`) {
      calls++
      if (options.delay) await new Promise(resolve => setTimeout(resolve, options.delay))
      if (options.failOnce && calls === 1) return route.fulfill({ status: 404, contentType: 'application/json', body: JSON.stringify({ error: '请求不存在或无权查看' }) })
      return send(value)
    }
    return route.fulfill({ status: 404, contentType: 'application/json', body: '{}' })
  })
  return () => calls
}

test('drawer shows phase timing and ordered attempts, preserves page, returns keyboard focus', async ({ page }, testInfo) => {
  const calls = await fixtures(page)
  await page.goto('/usage?range=7d&model=fixture-model')
  await page.getByRole('button', { name: '下一页' }).click()
  await expect(page.locator('.usage-page-number')).toHaveText('2 / 2')
  const trigger = page.getByRole('button', { name: '查看 fixture-model 请求详情' })
  await trigger.focus()
  await page.keyboard.press('Enter')
  const panel = page.getByRole('dialog', { name: '请求详情' })
  await expect(panel).toBeVisible()
  await expect(panel.getByText('前序渠道尝试', { exact: true })).toBeVisible()
  await expect(panel.locator('li').first()).toContainText('主渠道')
  await expect(panel.locator('li').first()).toContainText('上游错误 · 503')
  await expect(panel.locator('li').last()).toContainText('已完成 · 200')
  await expect(panel.getByText('126,788', { exact: true })).toBeAttached()
  await page.screenshot({ path: testInfo.outputPath('request-details-desktop.png'), animations: 'disabled' })
  await page.keyboard.press('Escape')
  await expect(panel).not.toBeVisible()
  await expect(trigger).toBeFocused()
  await expect(page.locator('.usage-page-number')).toHaveText('2 / 2')
  await expect(page).toHaveURL(/range=7d&model=fixture-model/)
  expect(calls()).toBe(1)
})

test('historical and non-stream requests do not invent first output or diagnostics', async ({ page }) => {
  await fixtures(page, { ...detail, stream: false, first_token_ms: null, request_diagnostics: null })
  await page.goto('/usage')
  await page.getByRole('button', { name: '查看 fixture-model 请求详情' }).click()
  const panel = page.getByRole('dialog')
  await expect(panel.getByText('收到响应头', { exact: true })).toBeVisible()
  await expect(panel.getByText('未采集分段计时。历史请求无法还原排队和渠道尝试耗时。')).toBeVisible()
  await expect(panel.getByText('未采集渠道尝试记录。')).toBeVisible()
  await expect(panel.getByText('首个输出 → 结束')).toHaveCount(0)
})

test('detail failure can be retried and loading has a readable state', async ({ page }) => {
  await fixtures(page, detail, { failOnce: true, delay: 250 })
  await page.goto('/usage')
  await page.getByRole('button', { name: '查看 fixture-model 请求详情' }).click()
  const panel = page.getByRole('dialog')
  await expect(panel.getByRole('status', { name: '正在加载' })).toBeVisible()
  await expect(panel.getByRole('alert')).toBeVisible()
  await panel.getByRole('button', { name: '重试', exact: true }).click()
  await expect(panel.getByText('渠道尝试', { exact: true })).toBeVisible()
})

test('mobile drawer handles interrupted streams and stays within the viewport', async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 })
  await fixtures(page, { ...detail, request_diagnostics: { ...detail.request_diagnostics!, completion: 'client_disconnected' } })
  await page.goto('/usage')
  await page.getByRole('button', { name: '查看 fixture-model 请求详情' }).click()
  const panel = page.getByRole('dialog')
  await expect(panel.getByText('客户端断开', { exact: true })).toBeVisible()
  await expect(panel.getByText('上游已返回成功状态，但本次响应未完整结束。')).toBeVisible()
  const box = await panel.boundingBox()
  expect(box!.width).toBeLessThanOrEqual(390)
  expect(box!.x).toBeGreaterThanOrEqual(0)
  expect(await panel.evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true)
  await page.screenshot({ path: testInfo.outputPath('request-details-mobile.png'), animations: 'disabled' })
  await panel.getByRole('button', { name: '关闭请求详情' }).click()
  await expect(panel).not.toBeVisible()
})
