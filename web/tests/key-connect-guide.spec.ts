import { expect, test, type Page } from '@playwright/test'
import type { VirtualKey } from '../src/lib/api'

const key: VirtualKey = { id: 'guide-key', name: '我的设备', key_prefix: 'test-ab12', can_copy: true, allowed_models: [], model_mode: 'all', known_models: ['claude-sonnet-test'], requests_per_minute: 60, token_limit_daily: null, cost_limit_daily_usd: null, cost_limit_total_usd: null, concurrency_limit: 4, enabled: true, created_at: '', revoked_at: null, used_tokens_today: 0, used_cost_today_usd: 0, used_cost_total_usd: 0, usage_reset_daily_at: null, usage_reset_total_at: null, channel_mode: 'all', channel_ids: [], channel_count: 0 }

async function setup(page: Page, keys: VirtualKey[] = []) {
  const state = { keys, reveals: 0, auth: [] as string[] }
  await page.route('**/api/**', async route => {
    const path = new URL(route.request().url()).pathname
    const method = route.request().method()
    const send = (body: unknown, status = 200) => route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) })
    if (path === '/api/me') return send({ user_id: 'test', email: 'test@example.invalid', account_id: 'account', is_super_admin: false })
    if (path === '/api/account') return send({ id: 'account', name: '接入验证', api_key_prefix: 'test' })
    if (path === '/api/channels' || path === '/api/model-catalog') return send([])
    if (path === '/api/keys' && method === 'GET') return send(state.keys)
    if (path === '/api/keys' && method === 'POST') {
      state.keys = [{ ...key, ...route.request().postDataJSON() }]
      return send({ id: key.id, key: 'test-ab12-created-mock' })
    }
    if (path.endsWith('/reveal')) { state.reveals++; return send({ key: 'test-ab12-revealed-mock' }) }
    return send({}, 404)
  })
  await page.route('**/v1/models', route => {
    state.auth.push(route.request().headers().authorization ?? '')
    return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ object: 'list', data: [{ id: 'claude-sonnet-test' }] }) })
  })
  return state
}

test('a new key leads straight into client setup and tests with the key just created', async ({ page, context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write'])
  const state = await setup(page)
  await page.goto('/keys?create=1')
  const dialog = page.getByRole('dialog', { name: '创建 API Key', exact: true })
  await expect(dialog).toBeVisible()
  await expect(page).toHaveURL(/\/keys$/)
  await dialog.getByLabel('Key 名称', { exact: true }).fill('我的设备')
  await dialog.getByRole('checkbox', { name: '允许全部模型', exact: true }).check()
  await dialog.getByRole('button', { name: '创建 API Key', exact: true }).click()
  const guide = page.getByRole('dialog', { name: 'API Key 已创建 · 我的设备' })
  await expect(guide.getByRole('region', { name: '保存完整 API Key' })).toContainText('test-ab12-created-mock')
  await expect(page.getByRole('heading', { name: '完整 API Key，请妥善保存' })).toHaveCount(0)
  await expect(guide.locator('pre')).toContainText('ANTHROPIC_AUTH_TOKEN="test-ab12-created-mock"')
  await expect(guide.getByRole('button', { name: '显示 Key' })).toHaveCount(0)
  await expect(guide).not.toContainText('Claude Code 默认请求')
  await guide.getByRole('button', { name: '复制配置' }).click()
  await expect(guide).toContainText('配置中包含完整 API Key')
  expect(await page.evaluate(() => navigator.clipboard.readText())).toContain('ANTHROPIC_AUTH_TOKEN="test-ab12-created-mock"')
  await guide.getByRole('button', { name: '测试连接' }).click()
  await expect(guide).toContainText('连接正常')
  expect(state.auth).toEqual(['Bearer test-ab12-created-mock'])
  expect(state.reveals).toBe(0)
  await guide.getByRole('button', { name: '复制 Key' }).click()
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe('test-ab12-created-mock')
  await page.keyboard.press('Escape')
  await expect(guide).toHaveCount(0)
  await expect(page.getByText('test-ab12-created-mock')).toHaveCount(0)
  await page.getByRole('button', { name: '如何使用 我的设备' }).click()
  const again = page.getByRole('dialog', { name: '接入客户端 · 我的设备' })
  await expect(again.getByRole('region', { name: '保存完整 API Key' })).toHaveCount(0)
  await expect(again.locator('pre .connect-key-token')).toBeVisible()
})

test('the row guide reveals a copyable key once and explains keys that cannot be tested', async ({ page }) => {
  const state = await setup(page, [key, { ...key, id: 'hashed-key', name: '旧 Key', key_prefix: 'test-old', can_copy: false, known_models: [] }])
  await page.goto('/keys')
  await page.getByRole('button', { name: '如何使用 我的设备' }).click()
  const sheet = page.getByRole('dialog', { name: '接入客户端 · 我的设备' })
  await expect(sheet.locator('pre .connect-key-token')).toBeVisible()
  await expect(sheet.locator('pre')).not.toContainText('revealed-mock')
  expect(state.reveals).toBe(0)
  await sheet.getByRole('button', { name: '显示 Key' }).click()
  await expect(sheet.locator('pre')).toContainText('ANTHROPIC_AUTH_TOKEN="test-ab12-revealed-mock"')
  await sheet.getByRole('button', { name: '隐藏 Key' }).click()
  await expect(sheet.locator('pre .connect-key-token')).toBeVisible()
  await sheet.getByRole('button', { name: '测试连接' }).click()
  await expect(sheet).toContainText('连接正常')
  await sheet.getByRole('button', { name: '测试连接' }).click()
  await expect(sheet).toContainText('连接正常')
  expect(state.reveals).toBe(1)
  await page.keyboard.press('Escape')
  await page.getByRole('button', { name: '如何使用 旧 Key' }).click()
  const old = page.getByRole('dialog', { name: '接入客户端 · 旧 Key' })
  await expect(old).toContainText('这把 Key 当前没有可调用的模型')
  await expect(old.locator('pre')).toContainText('ANTHROPIC_AUTH_TOKEN="你的 VAIR Key"')
  await expect(old.getByRole('button', { name: '显示 Key' })).toHaveCount(0)
  await old.getByRole('button', { name: '测试连接' }).click()
  await expect(old).toContainText('无法在网页中测试')
  expect(state.auth).toHaveLength(2)
})

test('the key page keeps the gateway address one click away', async ({ page, context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write'])
  await setup(page, [key])
  await page.goto('/keys')
  const origin = new URL(page.url()).origin
  const address = page.getByRole('region', { name: '接口地址' })
  await address.getByRole('button', { name: '复制 /v1 接口地址' }).click()
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(`${origin}/v1`)
  await address.getByRole('button', { name: '复制接口地址' }).click()
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(origin)
  await page.getByRole('button', { name: '查看接入指南' }).click()
  await expect(page.getByRole('dialog', { name: '接入客户端 · 我的设备' })).toBeVisible()
  await page.keyboard.press('Escape')
  await page.getByRole('button', { name: '创建 API Key', exact: true }).first().click()
  await expect(page.getByRole('dialog', { name: '创建 API Key', exact: true })).toContainText('Key 前缀可在帐号设置中自定义')
})

test('without a usable key the address stays but the guide button is hidden', async ({ page }) => {
  await setup(page)
  await page.goto('/keys')
  await expect(page.getByRole('region', { name: '接口地址' })).toBeVisible()
  await expect(page.getByRole('button', { name: '查看接入指南' })).toHaveCount(0)
})
