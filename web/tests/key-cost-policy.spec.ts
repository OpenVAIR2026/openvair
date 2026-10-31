import { expect, test, type Page } from '@playwright/test'
import type { Channel, VirtualKey } from '../src/lib/api'

const channel = (id: string, name: string, models: string[], extra: Partial<Channel> = {}): Channel => ({ id, name, protocol: 'openai', base_url: 'https://api.example.invalid', enabled: true, priority: 10, health_status: 'healthy', health_error: null, health_checked_at: null, test_model: null, models, model_mapping: {}, model_mode: 'allowlist', model_count: models.length, known_models: models, discovered_models: [], models_synced_at: null, created_at: '', ...extra })

const key: VirtualKey = { id: 'test-key', name: '费用策略验证', key_prefix: 'test', can_copy: false, allowed_models: [], model_mode: 'all', known_models: [], requests_per_minute: 60, token_limit_daily: null, cost_limit_daily_usd: 10, cost_limit_total_usd: null, concurrency_limit: 4, enabled: true, created_at: '', revoked_at: null, used_tokens_today: 0, used_cost_today_usd: 0, used_cost_total_usd: 0, usage_reset_daily_at: null, usage_reset_total_at: null, channel_mode: 'all', channel_ids: [] }

async function setup(page: Page, existing: boolean | VirtualKey = false) {
  const state = { keys: existing ? [{ ...key, ...(existing === true ? {} : existing) }] : [] as VirtualKey[], writes: [] as Record<string, unknown>[], resets: [] as { path: string; body: unknown }[], channels: [] as Channel[], failSave: false, listRefresh: null as Promise<void> | null }
  await page.route('**/api/**', async route => {
    const path = new URL(route.request().url()).pathname
    const method = route.request().method()
    const send = (body: unknown, status = 200) => route.fulfill({ status, contentType: 'application/json', body: status === 204 ? undefined : JSON.stringify(body) })
    if (path === '/api/me') return send({ user_id: 'test', email: 'test@example.invalid', account_id: 'account', is_super_admin: false })
    if (path === '/api/account') return send({ id: 'account', name: '费用策略验证', api_key_prefix: 'test' })
    if (path === '/api/channels') return send(state.channels)
    if (path === '/api/model-catalog') return send([])
    if (path === '/api/available-models') return send({})
    if (path === '/api/keys' && method === 'GET') {
      await state.listRefresh
      return send(state.keys)
    }
    if (path.endsWith('/usage-reset') && method === 'POST') {
      const body = route.request().postDataJSON() as { scope: string }
      state.resets.push({ path, body })
      state.keys = state.keys.map(item => ({ ...item, used_tokens_today: 0, used_cost_today_usd: 0, ...(body.scope === 'all' ? { used_cost_total_usd: 0 } : {}) }))
      return send(null, 204)
    }
    if (path.startsWith('/api/keys') && (method === 'POST' || method === 'PATCH')) {
      const input = route.request().postDataJSON()
      state.writes.push(input)
      if (state.failSave) return send({ error: { message: '保存失败，请重试' } }, 503)
      state.keys = [{ ...key, ...state.keys[0], ...input }]
      return method === 'POST' ? send({ id: key.id, key: 'test-only-mock-key' }) : send(null, 204)
    }
    return send({}, 404)
  })
  await page.goto('/keys')
  return state
}

test('new keys save limits without a retired cost policy', async ({ page }, testInfo) => {
  const state = await setup(page)
  await page.getByRole('button', { name: '创建 API Key', exact: true }).first().click()
  const dialog = page.getByRole('dialog', { name: '创建 API Key', exact: true })
  await expect(dialog.getByLabel('费用无法确定时', { exact: true })).toHaveCount(0)
  await dialog.getByLabel('Key 名称', { exact: true }).fill('新应用')
  await dialog.getByRole('checkbox', { name: '允许全部模型', exact: true }).check()
  await expect(dialog.getByLabel('允许模型', { exact: true })).toHaveCount(0)
  await expect(dialog.getByText('未添加限制，Token 和费用不受限。')).toBeVisible()
  await expect(dialog.getByLabel('每日费用上限（USD）', { exact: true })).toHaveCount(0)
  await dialog.getByRole('button', { name: '添加限制' }).click()
  await page.getByRole('menuitem', { name: '每日费用', exact: true }).click()
  await dialog.getByLabel('每日费用上限（USD）', { exact: true }).fill('5')
  await page.screenshot({ path: testInfo.outputPath('key-cost-policy-desktop.png'), fullPage: true })
  await dialog.getByRole('button', { name: '创建 API Key', exact: true }).click()
  await expect(dialog).not.toBeVisible()
  expect(state.writes[0]).toMatchObject({ cost_limit_daily_usd: 5, token_limit_daily: null, cost_limit_total_usd: null, requests_per_minute: 60, channel_mode: 'all', channel_ids: [] })
  expect(state.writes[0]).not.toHaveProperty('unknown_cost_policy')
})

test('editing keeps the dialog open on failed saves and saving waits for the refreshed key', async ({ page }) => {
  const state = await setup(page, true)
  await page.getByRole('button', { name: '编辑', exact: true }).click()
  const dialog = page.getByRole('dialog', { name: '编辑 API Key · 费用策略验证', exact: true })
  await dialog.getByRole('button', { name: '移除每日费用限制' }).click()
  state.failSave = true
  await dialog.getByRole('button', { name: '保存设置', exact: true }).click()
  await expect(dialog.getByRole('alert')).toContainText('保存失败')
  await expect(dialog.getByLabel('每日费用上限（USD）', { exact: true })).toHaveCount(0)
  state.failSave = false
  let finishRefresh!: () => void
  state.listRefresh = new Promise(resolve => { finishRefresh = resolve })
  const refreshing = page.waitForRequest(request => new URL(request.url()).pathname === '/api/keys' && request.method() === 'GET')
  try {
    await dialog.getByRole('button', { name: '保存设置', exact: true }).click()
    await refreshing
    await expect(dialog.getByRole('button', { name: '保存中…', exact: true })).toBeDisabled()
  } finally {
    finishRefresh()
  }
  await expect(dialog).not.toBeVisible()
  expect(state.writes.at(-1)).toMatchObject({ cost_limit_daily_usd: null })
  await page.getByRole('button', { name: '停用', exact: true }).click()
  await expect(page.getByRole('button', { name: '启用', exact: true })).toBeVisible()
  expect(state.writes.at(-1)).toEqual({ enabled: false })
})

test('the list shows daily and lifetime usage against limits and the total cap saves and clears', async ({ page }, testInfo) => {
  const state = await setup(page, { ...key, token_limit_daily: 5_000_000, used_tokens_today: 1_200_000, used_cost_today_usd: 8.5, cost_limit_total_usd: 100, used_cost_total_usd: 100 })
  const row = page.getByRole('row', { name: /费用策略验证/ })
  const meter = (name: string) => row.getByRole('meter', { name, exact: true })
  await expect(meter('今日 Token')).toHaveAttribute('aria-valuenow', '1200000')
  await expect(meter('今日 Token')).toHaveAttribute('aria-valuemax', '5000000')
  await expect(row.getByText('1.2M', { exact: true })).toBeVisible()
  await expect(row.getByText('/ 5M', { exact: true })).toBeVisible()
  await expect(row.getByText('$8.50', { exact: true })).toBeVisible()
  await expect(row.getByText('/ $10.00', { exact: true })).toBeVisible()
  await expect(meter('累计费用')).toHaveAttribute('aria-valuenow', '100')
  await expect(row.getByRole('img', { name: /未计价/ })).toHaveCount(0)
  await expect(row.getByText('已达上限', { exact: true })).toBeVisible()
  await expect(row.getByText('启用', { exact: true })).toHaveCount(0)
  await expect(row.getByRole('link', { name: '查看用量', exact: true })).toHaveAttribute('href', '/usage?key=test-key')
  await page.screenshot({ path: testInfo.outputPath('key-usage-desktop.png'), fullPage: true })
  await row.getByRole('button', { name: '编辑', exact: true }).click()
  const dialog = page.getByRole('dialog', { name: '编辑 API Key · 费用策略验证', exact: true })
  const total = dialog.getByLabel('总费用上限（USD）', { exact: true })
  await expect(total).toHaveValue('100')
  await total.fill('250')
  await dialog.getByRole('button', { name: '保存设置', exact: true }).click()
  await expect(dialog).not.toBeVisible()
  expect(state.writes.at(-1)).toMatchObject({ cost_limit_total_usd: 250 })
  await row.getByRole('button', { name: '编辑', exact: true }).click()
  await total.fill('')
  await dialog.getByRole('button', { name: '保存设置', exact: true }).click()
  await expect(dialog.getByText('请输入上限，或移除此限制')).toBeVisible()
  await dialog.getByRole('button', { name: '移除总费用限制' }).click()
  await dialog.getByRole('button', { name: '保存设置', exact: true }).click()
  await expect(dialog).not.toBeVisible()
  expect(state.writes.at(-1)).toMatchObject({ cost_limit_total_usd: null, token_limit_daily: 5_000_000 })
})

test('usage resets ask for confirmation and send the chosen scope', async ({ page }, testInfo) => {
  const state = await setup(page, { ...key, token_limit_daily: 1_000_000, used_tokens_today: 1_000_000, used_cost_today_usd: 3, cost_limit_total_usd: 20, used_cost_total_usd: 20 })
  const row = page.getByRole('row', { name: /费用策略验证/ })
  await expect(row.getByText('已达上限', { exact: true })).toBeVisible()
  const resetToday = row.getByRole('button', { name: '重置今日用量 费用策略验证' })
  await row.getByRole('button', { name: '更多操作 费用策略验证' }).click()
  await page.screenshot({ path: testInfo.outputPath('key-actions-menu.png') })
  await expect(page.getByRole('menuitem', { name: '重置今日用量' })).toHaveCount(0)
  await page.keyboard.press('Escape')
  await resetToday.click()
  const confirm = page.getByRole('alertdialog', { name: '重置今日用量？' })
  await expect(confirm).toContainText('累计费用不变')
  await expect(confirm).toContainText('今日已用 1M Token、$3.00')
  await page.screenshot({ path: testInfo.outputPath('key-reset-confirm.png') })
  await confirm.getByRole('button', { name: '取消' }).click()
  expect(state.resets).toHaveLength(0)
  await resetToday.click()
  await page.getByRole('alertdialog').getByRole('button', { name: '确认' }).click()
  await expect(page.getByRole('alertdialog')).toHaveCount(0)
  expect(state.resets.at(-1)).toEqual({ path: '/api/keys/test-key/usage-reset', body: { scope: 'today' } })
  await expect(row.getByText('已达上限', { exact: true })).toBeVisible()
  // Nothing is left to reset today, so the action stays in place but is disabled.
  await expect(resetToday).toBeDisabled()
  await row.getByRole('button', { name: '更多操作 费用策略验证' }).click()
  await page.getByRole('menuitem', { name: '重置全部用量' }).click()
  await page.getByRole('alertdialog', { name: '重置全部用量？' }).getByRole('button', { name: '确认' }).click()
  await expect(page.getByRole('alertdialog')).toHaveCount(0)
  expect(state.resets.at(-1)?.body).toEqual({ scope: 'all' })
  await expect(row.getByText('启用', { exact: true })).toBeVisible()
  await row.getByRole('button', { name: '更多操作 费用策略验证' }).click()
  await page.getByRole('menuitem', { name: '撤销' }).click()
  await expect(page.getByRole('alertdialog', { name: '撤销 API Key？' })).toBeVisible()
})

test('the key list keeps meters and actions aligned across mixed keys', async ({ page }, testInfo) => {
  const state = await setup(page, true)
  state.channels = [channel('primary', '主渠道', ['gpt-4.1']), channel('backup', '备用渠道', ['claude-sonnet'])]
  state.keys = [
    { ...key, id: 'production', name: '生产应用', can_copy: true, channel_mode: 'selected', channel_ids: ['primary', 'backup'], model_mode: 'allowlist', allowed_models: ['gpt-*', 'claude-*'], known_models: ['gpt-4.1', 'claude-sonnet'], token_limit_daily: 20_000_000, used_tokens_today: 6_400_000, cost_limit_daily_usd: 50, used_cost_today_usd: 12.4, cost_limit_total_usd: 1000, used_cost_total_usd: 318.6 },
    { ...key, id: 'batch', name: '批处理任务', can_copy: true, requests_per_minute: null, concurrency_limit: 16, cost_limit_daily_usd: null, used_tokens_today: 860_000, used_cost_today_usd: 3.2, used_cost_total_usd: 41.75 },
    { ...key, id: 'trial', name: '试用客户', can_copy: true, cost_limit_daily_usd: 5, used_cost_today_usd: 4.3, cost_limit_total_usd: 20, used_cost_total_usd: 20 },
    { ...key, id: 'legacy', name: '旧版集成', enabled: false, used_cost_total_usd: 7.9, channel_mode: 'selected', channel_ids: [] },
  ]
  await page.reload()
  await expect(page.getByRole('row')).toHaveCount(5)
  await expect(page.getByRole('row', { name: /生产应用/ })).toContainText('主渠道、备用渠道')
  await expect(page.getByRole('row', { name: /旧版集成/ })).toContainText('未接入可用渠道')
  await expect(page.getByRole('row', { name: /批处理任务/ })).toContainText('全部渠道')
  await expect(page.getByRole('row', { name: /批处理任务/ })).toContainText(/请求\s*不限\s*并发\s*16/)
  await expect(page.getByRole('row', { name: /生产应用/ })).toContainText(/请求\s*60\s*\/分\s*并发\s*4/)
  await page.screenshot({ path: testInfo.outputPath('key-list-desktop.png'), fullPage: true })
  await page.getByRole('row', { name: /生产应用/ }).getByRole('button', { name: '编辑', exact: true }).click()
  await page.screenshot({ path: testInfo.outputPath('key-form-allowlist.png') })
  await page.getByRole('checkbox', { name: '允许全部模型', exact: true }).check()
  await page.screenshot({ path: testInfo.outputPath('key-form-all.png') })
})

test('keys choose channels first and model suggestions follow the chosen channels', async ({ page }, testInfo) => {
  const state = await setup(page)
  state.channels = [channel('primary', '主渠道', ['only-a-model']), channel('backup', '备用渠道', ['only-b-model'])]
  await page.reload()
  await page.getByRole('button', { name: '创建 API Key', exact: true }).first().click()
  const dialog = page.getByRole('dialog', { name: '创建 API Key', exact: true })
  const allChannels = dialog.getByRole('checkbox', { name: '允许全部渠道', exact: true })
  await expect(allChannels).toBeChecked()
  await dialog.getByLabel('Key 名称', { exact: true }).fill('限定渠道')
  await allChannels.uncheck()
  const models = dialog.getByLabel('允许模型', { exact: true })
  await models.fill('only-a-model')
  await models.press('Enter')
  await dialog.getByRole('button', { name: '创建 API Key', exact: true }).click()
  await expect(dialog.getByText('至少选择一个渠道，或勾选允许全部渠道')).toBeVisible()
  expect(state.writes).toHaveLength(0)
  await dialog.getByRole('checkbox', { name: '备用渠道', exact: true }).check()
  await expect(dialog.getByText('所选渠道中无可用模型：only-a-model')).toBeVisible()
  await page.getByRole('button', { name: '移除模型 only-a-model', exact: true }).click()
  await models.fill('only-')
  await expect(page.getByRole('option', { name: /only-b-model/ })).toBeVisible()
  await expect(page.getByRole('option', { name: /only-a-model/ })).toHaveCount(0)
  await page.screenshot({ path: testInfo.outputPath('key-form-channels.png') })
  await models.fill('only-b-model')
  await models.press('Enter')
  await expect(dialog.getByText('所选渠道中无可用模型', { exact: false })).toHaveCount(0)
  await dialog.getByRole('button', { name: '创建 API Key', exact: true }).click()
  await expect(dialog).not.toBeVisible()
  expect(state.writes[0]).toMatchObject({ channel_mode: 'selected', channel_ids: ['backup'], model_mode: 'allowlist', allowed_models: ['only-b-model'] })
})

test('the key list scrolls inside its card on phone widths without widening the page', async ({ page }) => {
  await setup(page, true)
  const scroller = page.locator('table').locator('xpath=..')
  await expect(page.getByRole('button', { name: '编辑', exact: true })).toBeVisible()
  for (const width of [1440, 390, 320]) {
    await page.setViewportSize({ width, height: 1000 })
    await expect.poll(() => page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
    if (width < 768) expect(await scroller.evaluate(el => el.scrollWidth > el.clientWidth)).toBe(true)
  }
})
