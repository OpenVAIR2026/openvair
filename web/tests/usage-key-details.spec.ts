import { expect, test } from '@playwright/test'

test('key holders see timing and usage without channel attempts, on desktop and mobile', async ({ page }, testInfo) => {
  await page.addInitScript(() => sessionStorage.setItem('vair.holder-key', 'mock-key-details-test-only'))
  const event = { id: 'key-event', created_at: '2026-10-14T03:54:22Z', model: 'fixture-model', protocol: 'openai', status: 200, ttfb_ms: 900, first_token_ms: 1000, stream: true, duration_ms: 1600, input_tokens: 126788, output_tokens: 424, cached_tokens: 121216, cost_usd: 0.027506, pricing_status: 'priced', source: 'gateway' }
  let details = 0
  await page.route('**/api/**', async route => {
    const url = new URL(route.request().url())
    const send = (body: unknown) => route.fulfill({ contentType: 'application/json', body: JSON.stringify(body) })
    if (url.pathname === '/api/key/summary') return send({ key_name: '开发 Key', status: 'active' })
    if (url.pathname === '/api/key/usage') return send({ totals: { requests: 1, successful: 1, input_tokens: 126788, output_tokens: 424, cached_tokens: 121216, known_cost_usd: 0.027506, avg_duration_ms: 1600 }, bucket_seconds: 3600, models: [], trend: [] })
    if (url.pathname === '/api/key/events') return send({ items: [event], total: 1, page: 1, page_size: 10 })
    if (url.pathname === '/api/key/events/key-event') {
      details++
      expect(route.request().headers().authorization).toBe('Bearer mock-key-details-test-only')
      return send({ ...event, timing: { queue_ms: 100, upstream_headers_ms: 750, headers_to_first_token_ms: 100, completion: 'success' } })
    }
    return route.fulfill({ status: 404, body: '{}' })
  })
  await page.goto('/my-key/usage')
  const trigger = page.getByRole('button', { name: '查看 fixture-model 请求详情' })
  await trigger.click()
  const panel = page.getByRole('dialog', { name: '请求详情' })
  await expect(panel.getByText('耗时分解', { exact: true })).toBeVisible()
  await expect(panel.getByText('750 ms', { exact: true })).toBeVisible()
  await expect(panel.getByText('126,788', { exact: true })).toBeVisible()
  await expect(panel.getByText('渠道', { exact: false })).toHaveCount(0)
  await expect(panel.getByText('尝试', { exact: false })).toHaveCount(0)
  await expect(panel.getByText('上游模型', { exact: false })).toHaveCount(0)
  await page.screenshot({ path: testInfo.outputPath('key-request-details-desktop.png'), animations: 'disabled' })
  await page.keyboard.press('Escape')
  await expect(trigger).toBeFocused()
  await page.setViewportSize({ width: 390, height: 844 })
  await trigger.click()
  await expect(panel).toBeVisible()
  expect(await panel.evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true)
  await page.screenshot({ path: testInfo.outputPath('key-request-details-mobile.png'), animations: 'disabled' })
  await panel.getByRole('button', { name: '关闭请求详情' }).click()
  await expect(panel).not.toBeVisible()
  expect(details).toBeGreaterThan(0)
})
