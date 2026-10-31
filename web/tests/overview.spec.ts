import { expect, test, type Page } from '@playwright/test'
import type { MonitoringChannel } from '../src/lib/api'
import { overviewChannelState } from '../src/lib/overview'

const channelId = (index: number) => `00000000-0000-4000-8000-${String(index).padStart(12, '0')}`
const model = 'demo-model'

function channels(): MonitoringChannel[] {
  const checked = new Date(Date.now() - 600_000).toISOString()
  return Array.from({ length: 6 }, (_, index) => ({
    id: channelId(index), name: index === 4 ? '第五个故障渠道' : index === 5 ? '已停用备用渠道' : `正常渠道 ${index + 1}`,
    protocol: 'openai', enabled: index !== 5, priority: index, models: [model], test_model: model,
    availability_enabled: true, availability_interval_seconds: 300,
    availability_status: index >= 4 ? 'unhealthy' : 'healthy', availability_error: index >= 4 ? 'HTTP 503' : null, availability_checked_at: checked,
    quality_enabled: false, quality_model: null, quality_models: [], quality_methods: [], quality_interval_seconds: 3600,
    quality_timeout_ms: 30000, quality_min_score: 80, quality_failure_threshold: 2, quality_daily_budget_micros: 0,
    quality_status: 'disabled', quality_last_observation: 'unknown', quality_score: null, quality_error: null, quality_checked_at: null,
    quality_consecutive_failures: 0, quality_valid_until: null, next_quality_at: null, effective_quality_models: [],
  }))
}

async function fixtures(page: Page) {
  // `fresh`: channels and a key exist, but no request has been made yet.
  const state = { empty: false, fresh: false, failures: new Set<string>(), requests: [] as URL[] }
  const channelRows = channels()
  await page.route('**/api/**', async route => {
    const url = new URL(route.request().url())
    state.requests.push(url)
    const path = url.pathname
    const send = (data: unknown, status = 200) => route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(data) })
    if (state.failures.has(path)) return send({ error: { message: '模拟服务暂时不可用' } }, 503)
    if (path === '/api/me') return send({ user_id: 'demo', email: 'demo@example.invalid', account_id: 'demo-account', is_super_admin: false })
    if (path === '/api/account') return send({ id: 'demo-account', name: '设计验证', channel_limit: 10, virtual_key_limit: 10, api_key_prefix: 'demo' })
    if (path === '/api/channels') return send(state.empty ? [] : channelRows)
    if (path === '/api/keys') return send(state.empty ? [] : [{ id: 'demo-key', name: '我的设备', key_prefix: 'demo-ab12', can_copy: true, known_models: [model], enabled: true, revoked_at: null }])
    if (path === '/api/keys/demo-key/reveal') return send({ key: 'demo-ab12-mock-secret' })
    if (path === '/api/monitoring/summary') return send({ total_channels: state.empty ? 0 : 6, available_channels: state.empty ? 0 : 4, quality_enabled: 0, quality_degraded: 0, channels: state.empty ? [] : channelRows })
    if (path === '/api/quality-monitors') return send([])
    if (path === '/api/monitoring/runs') return send(url.searchParams.has('page') ? { items: [], total: 0, page: 1, page_size: 10 } : [])
    const range = url.searchParams.get('range') ?? '24h'
    const requests = state.empty || state.fresh ? 0 : range === '7d' ? 24000 : range === '30d' ? 96000 : 2400
    const successful = Math.round(requests * .94)
    if (path === '/api/usage/summary') {
      const count = range === '24h' ? 24 : range === '7d' ? 7 : 30
      const step = range === '24h' ? 3600 : 86400
      const end = new Date()
      if (range === '24h') end.setMinutes(0, 0, 0); else end.setHours(0, 0, 0, 0)
      const trend = Array.from({ length: count }, (_, index) => ({ start: new Date(end.getTime() - (count - 1 - index) * step * 1000).toISOString(), requests: state.empty || index === 1 ? 0 : index === 0 ? Math.floor(requests / (count - 1)) + requests % (count - 1) : Math.floor(requests / (count - 1)), tokens: 0 }))
      return send({
        since: trend[0].start, bucket_seconds: step,
        totals: { requests, successful, input_tokens: 0, output_tokens: 0, cached_tokens: 0, active_models: state.empty ? 0 : 1, avg_duration_ms: state.empty ? null : 1800, avg_ttfb_ms: state.empty ? null : 340, cache_reported_requests: 0, cache_unreported_requests: requests, cache_unknown_requests: 0, cache_reported_input_tokens: 0, cache_reported_tokens: 0 },
        trend, models: state.empty ? [] : [{ model, requests, tokens: 0, estimated_cost_usd: null }], channels: [], model_options: [model],
      })
    }
    if (path === '/api/usage/events') {
      let items = [503, 200, 200].map((status, index) => ({ id: `demo-${index}`, channel_id: channelId(4), channel_name: '第五个故障渠道', model, protocol: 'openai', status, ttfb_ms: status === 200 ? 340 : null, duration_ms: 1800, input_tokens: null, output_tokens: null, cached_tokens: null, created_at: new Date(Date.now() - index * 60_000).toISOString() }))
      if (url.searchParams.get('status') === 'error') items = items.filter(item => item.status !== 200)
      return send({ items: state.empty || state.fresh ? [] : items, total: state.empty || state.fresh ? 0 : url.searchParams.get('status') === 'error' ? requests - successful : requests, page: 1, page_size: Number(url.searchParams.get('limit')) })
    }
    return send({ error: { message: `Unexpected mock request: ${path}` } }, 404)
  })
  return state
}

const metric = (page: Page, label: string) => page.locator('.overview-metric').filter({ has: page.locator('dt', { hasText: label }) })

test('uses complete server totals and time buckets, with failures ranked first', async ({ page }) => {
  const state = await fixtures(page)
  await page.goto('/overview?range=24h')
  await expect(metric(page, '请求量').locator('.overview-metric-value')).toHaveText('2,400')
  await expect(metric(page, '成功率')).toContainText('94.0%')
  await expect(page.locator('.overview-health-row').first()).toContainText('第五个故障渠道')
  await expect(page.locator('.overview-signal')).toContainText('1 个渠道需要关注')
  await expect(page.locator('.overview-health-card .overview-card-heading')).toContainText('可用性正常 4 / 5')
  await expect(page.locator('.overview-health-card .overview-card-heading')).toContainText('已停用 1')
  await expect(page.locator('.overview-events-table')).toContainText('503 失败')
  await expect(page.locator('.overview-events-table')).toContainText('未上报')
  expect(state.requests.some(url => url.pathname === '/api/usage')).toBe(false)
  expect(state.requests.find(url => url.pathname === '/api/usage/summary')?.searchParams.get('utc_offset_minutes')).toBe('480')
})

test('range and failure links persist through analysis navigation and reload', async ({ page }) => {
  const state = await fixtures(page)
  await page.goto('/overview')
  await page.getByRole('combobox', { name: '统计范围' }).click()
  await page.getByRole('option', { name: '近 7 天' }).click()
  await expect(metric(page, '请求量')).toContainText('24,000')
  await page.getByRole('button', { name: '仅失败', exact: true }).click()
  await expect(page.locator('.overview-events-table tbody tr')).toHaveCount(1)
  await expect(page).toHaveURL(/range=7d&status=error/)
  await page.locator('.overview-request-link').click()
  await expect(page.getByRole('heading', { name: '使用分析', exact: true })).toBeVisible()
  await expect(page.getByRole('combobox', { name: '数据范围' })).toContainText('近 7 天')
  await expect.poll(() => state.requests.filter(url => url.pathname === '/api/usage/events' && url.searchParams.get('limit') === '10').at(-1)?.searchParams.toString()).toContain('channel_id=' + channelId(4))
  const last = state.requests.filter(url => url.pathname === '/api/usage/events' && url.searchParams.get('limit') === '10').at(-1)!
  expect(last.searchParams.get('range')).toBe('7d')
  expect(last.searchParams.get('status')).toBe('error')
  expect(last.searchParams.get('model')).toBe(model)
  await page.reload()
  await expect(page.getByRole('combobox', { name: '数据范围' })).toContainText('近 7 天')
  await expect(page.getByRole('combobox', { name: '请求状态' })).toContainText('异常请求')
})

test('monitoring drilldown scopes both the channel list and run history', async ({ page }) => {
  const state = await fixtures(page)
  await page.goto('/overview')
  await page.getByRole('link', { name: '查看异常详情' }).click()
  await expect(page).toHaveURL(new RegExp('channel=' + channelId(4)))
  await expect(page.getByText('当前渠道：第五个故障渠道')).toBeVisible()
  await expect(page.locator('#monitor-panel-availability table tbody tr')).toHaveCount(1)
  await expect.poll(() => state.requests.filter(url => url.pathname === '/api/monitoring/runs' && url.searchParams.has('page')).at(-1)?.searchParams.get('channel_id')).toBe(channelId(4))
  await page.getByRole('button', { name: '查看全部渠道', exact: true }).click()
  await expect(page.locator('#monitor-panel-availability table tbody tr')).toHaveCount(6)
  await expect.poll(() => state.requests.filter(url => url.pathname === '/api/monitoring/runs' && url.searchParams.has('page')).at(-1)?.searchParams.has('channel_id')).toBe(false)
})

test('no requests show neutral unknown rates and latency, not failed requests', async ({ page }) => {
  const state = await fixtures(page)
  state.empty = true
  await page.goto('/overview')
  await expect(metric(page, '请求量').locator('.overview-metric-value')).toHaveText('0')
  await expect(metric(page, '成功率').locator('.overview-metric-value')).toHaveText('—')
  await expect(metric(page, '平均首字延迟').locator('.overview-metric-value')).toHaveText('—')
  await expect(page.getByRole('heading', { name: '开始使用' })).toBeVisible()
  await expect(page.locator('.overview-signal')).toHaveCount(0)
  await expect(page.getByText('当前时段暂无请求', { exact: true })).toBeVisible()
})

test('initial usage failure does not invent zeros and can be retried independently', async ({ page }) => {
  const state = await fixtures(page)
  state.failures.add('/api/usage/summary')
  await page.goto('/overview')
  await expect(metric(page, '请求量').locator('.overview-metric-value')).toHaveText('—')
  await expect(page.getByText('当前时段暂无请求', { exact: true })).toHaveCount(0)
  await expect(page.locator('.overview-health-row').first()).toContainText('第五个故障渠道')
  await expect(page.locator('.overview-events-table tbody tr')).toHaveCount(3)
  state.failures.clear()
  await page.getByRole('button', { name: '重试', exact: true }).click()
  await expect(metric(page, '请求量').locator('.overview-metric-value')).toHaveText('2,400')
})

test('refresh failures preserve previous values and distinguish stale snapshots', async ({ page }) => {
  const state = await fixtures(page)
  await page.goto('/overview')
  await expect(metric(page, '请求量').locator('.overview-metric-value')).toHaveText('2,400')
  await expect(page.getByRole('button', { name: '刷新数据' })).toBeEnabled()
  state.failures.add('/api/usage/summary')
  state.failures.add('/api/monitoring/summary')
  await page.getByRole('button', { name: '刷新数据' }).click()
  await expect(page.getByRole('alert').filter({ hasText: '刷新失败，保留上次数据' })).toHaveCount(2)
  await expect(metric(page, '请求量').locator('.overview-metric-value')).toHaveText('2,400')
  await expect(page.locator('.overview-health-row').first()).toContainText('第五个故障渠道')
  await expect(page.getByRole('heading', { name: '渠道状态刷新失败' })).toBeVisible()
})

test('layout keeps exception visible without page overflow on desktop and phones', async ({ page }, testInfo) => {
  await fixtures(page)
  await page.goto('/overview')
  await expect(page.locator('.overview-health-row').first()).toBeVisible()
  for (const width of [1440, 390, 320]) {
    await page.setViewportSize({ width, height: 1000 })
    await expect.poll(() => page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
    const box = await page.locator('.overview-signal').boundingBox()
    expect(box!.y + box!.height).toBeLessThan(844)
    const status = await page.locator('.overview-events-table tbody tr').first().locator('td').nth(2).boundingBox()
    expect(status!.x + status!.width).toBeLessThanOrEqual(width)
    await page.screenshot({ path: testInfo.outputPath(`overview-${width}.png`), fullPage: true })
  }
})

test('disabled, unmonitored and degraded-quality channels remain distinct', () => {
  const [channel] = channels()
  expect(overviewChannelState({ ...channel, enabled: false, availability_status: 'unhealthy' })).toMatchObject({ label: '已停用', priority: 3, tone: 'neutral' })
  expect(overviewChannelState({ ...channel, availability_enabled: false, availability_status: 'unhealthy' })).toMatchObject({ label: '未监控可用性', tone: 'neutral' })
  expect(overviewChannelState({ ...channel, availability_status: 'unknown' })).toMatchObject({ label: '待检查', priority: 1 })
  const expired = { channel_id: channel.id, model: 'model-a', status: 'degraded', checked_at: channel.availability_checked_at, valid_until: new Date(0).toISOString(), last_error: null, schedule_enabled: true }
  // The latest result stands even after its validity ends.
  expect(overviewChannelState(channel, [expired])).toMatchObject({ label: '质量异常', kind: 'quality', priority: 0 })
  // Manual-only monitors do not ask for attention while waiting for the next manual run.
  const manual = { ...expired, status: 'healthy', schedule_enabled: false }
  expect(overviewChannelState(channel, [manual, { ...manual, model: 'model-c', status: 'unknown', checked_at: null, valid_until: null }])).toMatchObject({ label: '正常' })
  const degraded = { ...expired, model: 'model-b', valid_until: '2099-01-01T00:00:00Z' }
  expect(overviewChannelState(channel, [manual, degraded])).toMatchObject({ label: '质量异常', kind: 'quality', priority: 0, detail: 'model-b：质量检查已达到异常确认阈值' })
  expect(overviewChannelState(channel, [{ ...degraded, channel_id: 'another-channel' }])).toMatchObject({ label: '正常' })
})

test('getting started leads from an empty account to adding a channel and can be hidden', async ({ page }) => {
  const state = await fixtures(page)
  state.empty = true
  await page.goto('/overview')
  const card = page.locator('.getting-started')
  await expect(card).toContainText('已完成 0 / 3')
  await expect(card.locator('li[aria-current="step"]')).toContainText('添加渠道')
  await expect(card.getByRole('button', { name: '查看接入方式' })).toBeDisabled()
  await expect(card.getByRole('link', { name: '添加渠道' })).toHaveAttribute('href', '/channels?add=1')
  await card.getByRole('button', { name: '隐藏开始使用' }).click()
  await expect(card).toHaveCount(0)
  await page.reload()
  await expect(metric(page, '请求量').locator('.overview-metric-value')).toHaveText('0')
  await expect(card).toHaveCount(0)
})

test('getting started is hidden once requests exist', async ({ page }) => {
  const state = await fixtures(page)
  await page.goto('/overview')
  await expect(metric(page, '请求量').locator('.overview-metric-value')).toHaveText('2,400')
  await expect(page.locator('.getting-started')).toHaveCount(0)
  expect(state.requests.find(url => url.pathname === '/api/usage/events' && url.searchParams.get('range') === 'all')?.searchParams.get('limit')).toBe('1')
})

test('the connect guide fills in the key and completes setup after a successful test', async ({ page }) => {
  const state = await fixtures(page)
  state.fresh = true
  const auth: string[] = []
  await page.route('**/v1/models', route => { auth.push(route.request().headers().authorization ?? ''); return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ object: 'list', data: [{ id: model }] }) }) })
  await page.goto('/overview')
  const card = page.locator('.getting-started')
  await expect(card).toContainText('已完成 2 / 3')
  await card.getByRole('button', { name: '查看接入方式' }).click()
  const guide = page.getByRole('dialog', { name: '接入客户端' })
  await expect(guide.locator('pre .connect-key-token')).toHaveText('复制时自动填入 Key')
  await expect(guide).not.toContainText('demo-ab12')
  await expect(guide.locator('pre')).not.toContainText('mock-secret')
  await expect(guide).toContainText('Claude Code 默认请求 claude-* 模型')
  await guide.getByRole('tab', { name: 'Codex' }).click()
  await expect(guide.locator('pre')).toContainText(`model = "${model}"`)
  await expect(guide).not.toContainText('Claude Code 默认请求')
  await guide.getByRole('button', { name: '测试连接' }).click()
  await expect(guide).toContainText('连接正常，这把 Key 可调用 1 个模型')
  expect(auth).toEqual(['Bearer demo-ab12-mock-secret'])
  await page.keyboard.press('Escape')
  await expect(card).toHaveCount(0)
})

test('a rejected key explains why the connection failed', async ({ page }) => {
  const state = await fixtures(page)
  state.fresh = true
  await page.route('**/v1/models', route => route.fulfill({ status: 401, contentType: 'application/json', body: '{"error":{"code":"unauthorized"}}' }))
  await page.goto('/overview')
  await page.locator('.getting-started').getByRole('button', { name: '查看接入方式' }).click()
  const guide = page.getByRole('dialog', { name: '接入客户端' })
  await guide.getByRole('button', { name: '测试连接' }).click()
  await expect(guide).toContainText('Key 无效、已停用或已撤销')
  await page.keyboard.press('Escape')
  await expect(page.locator('.getting-started')).toContainText('已完成 2 / 3')
})

test('the sidebar logo returns to the homepage and reacts on hover', async ({ page }) => {
  await fixtures(page)
  await page.goto('/overview')
  const brand = page.locator('.sidebar-brand-header').getByRole('link', { name: 'VAIR 首页' })
  const mark = brand.locator('.brand-mark')
  const before = await mark.evaluate(element => getComputedStyle(element).transform)
  await brand.hover()
  await expect.poll(() => mark.evaluate(element => getComputedStyle(element).transform)).not.toBe(before)
  await brand.click()
  await expect(page).toHaveURL(/\/$/)
})
