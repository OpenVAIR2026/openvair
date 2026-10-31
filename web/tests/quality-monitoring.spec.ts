import { expect, test, type Page } from '@playwright/test'

const channel = { id: '11111111-1111-4111-8111-111111111111', name: '测试渠道', enabled: true, protocol: 'anthropic', priority: 100, models: ['model-a', 'model-b'], known_models: ['model-a', 'model-b'], discovered_models: [] as string[], model_mapping: { 'model-a': 'upstream-a' } as Record<string, string>, test_model: 'model-a', model_mode: 'allowlist', availability_status: 'healthy', availability_enabled: true, availability_interval_seconds: 1800, availability_checked_at: '2026-10-07T09:00:00Z' }
const settings = { schedule_enabled: false, interval_seconds: 3600, timeout_ms: 60000, round_timeout_ms: 240000, methods: ['integrity', 'metering', 'tools'], min_score: 75, failure_threshold: 2, daily_run_limit: 0, route_policy: 'observe' }
const monitor = { ...settings, kind: 'service', identity_verdict: null, identity_checked_at: null, identity_evidence: null, last_tests: null, claimed_model: null, samples_per_run: 2, window_days: 7, daily_sample_limit: 0, window_verdict: null, window_evidence: null, window_checked_at: null, window_valid_until: null, id: '22222222-2222-4222-8222-222222222222', channel_id: channel.id, channel_name: channel.name, channel_enabled: true, protocol: channel.protocol, model: 'model-a', upstream_model: 'upstream-a', status: 'healthy', last_observation: 'healthy', score: 100, consecutive_failures: 0, last_error: null, checked_at: '2026-10-07T09:00:00Z', valid_until: '2099-10-07T11:00:00Z', next_run_at: null, schedule_error: null, config_version: 1, job_status: 'completed', job_error: null }
let baselineModels: any[] = []
let historyItems: any[] = []
let passiveRows: any[] = []
let runItems: any[] = []
async function setup(page: Page, populated = false, channelRows = [channel], seeded: any[] = [monitor]) {
  let monitors: any[] = populated ? seeded.map(m => ({ ...m })) : []
  const writes: { method: string; path: string; body: any }[] = []
  await page.route('**/api/**', async route => {
    const url = new URL(route.request().url())
    const path = url.pathname.replace('/api', '')
    const method = route.request().method()
    let body: any = {}
    if (method !== 'GET') writes.push({ method, path, body: route.request().postDataJSON() })
    if (path === '/me') body = { user_id: 'user', email: 'test@example.invalid', account_id: 'account', is_super_admin: false }
    else if (path === '/account') body = { id: 'account', name: '测试帐号' }
    else if (path === '/channels') body = channelRows
    else if (path === '/monitoring/summary') body = { total_channels: channelRows.length, available_channels: channelRows.length, channels: channelRows }
    else if (path === '/monitoring/runs') body = url.searchParams.has('per_channel') ? [] : { items: runItems, total: runItems.length, page: 1, page_size: 10 }
    else if (path === '/passive-metering') body = passiveRows
    else if (path.endsWith('/long-context') && method === 'GET') body = { tiers: [{ tokens: 8000, cost_usd: 0.02 }, { tokens: 32000, cost_usd: 0.1 }, { tokens: 128000, cost_usd: 0.42 }], upstream_model: 'upstream-a', acknowledged: false, active: null, max_passed: null, checked_at: null }
    else if (path.endsWith('/long-context') && method === 'POST') body = { job_id: 'job' }
    else if (path === '/quality-monitors' && method === 'GET') body = monitors
    else if (path === '/quality-monitors' && method === 'POST') { const data = route.request().postDataJSON(); const id = data.kind === 'authenticity' ? '33333333-3333-4333-8333-333333333333' : monitor.id; monitors = [...monitors, { ...monitor, ...data.settings, id, kind: data.kind ?? 'service', model: data.model }]; body = { id, job_id: data.run_immediately && data.kind !== 'authenticity' ? 'job' : null } }
    else if (path.endsWith('/run') && method === 'POST') { monitors = monitors.map(m => ({ ...m, job_status: 'queued' })); body = { job_id: 'job' } }
    else if (path.startsWith('/quality-monitors/') && method === 'PATCH') { monitors = monitors.map(m => ({ ...m, ...route.request().postDataJSON() })); body = { id: monitor.id } }
    else if ((path.endsWith('/deep-check') || path === '/authenticity-options') && method === 'GET') body = { monitor_id: monitor.id, model: 'model-a', upstream_model: 'upstream-a', claimed_default: null, acknowledged: false, tiers: [16, 32, 64], min_samples: 8, max_samples: 256, daily_sample_limit: 0, models: baselineModels }
    else if (path.endsWith('/deep-checks')) body = []
    else if (path === '/authenticity-checks') body = { items: historyItems, total: historyItems.length, page: 1, page_size: 10 }
    else if (path.startsWith('/quality-monitors/') && method === 'DELETE') { monitors = []; await route.fulfill({ status: 204 }); return }
    else throw new Error(`Unexpected API request: ${method} ${path}`)
    await route.fulfill({ json: body })
  })
  await page.goto('/monitoring?tab=quality')
  return writes
}

test('create one model monitor, run with scheduling off, and retain history on delete', async ({ page }) => {
  const writes = await setup(page)
  await page.getByRole('button', { name: '添加服务体检', exact: true }).first().click()
  const dialog = page.getByRole('dialog', { name: '添加服务体检' })
  await dialog.getByLabel('渠道', { exact: true }).click()
  await page.getByRole('option', { name: '测试渠道', exact: true }).click()
  await dialog.getByLabel('模型', { exact: true }).click()
  await page.locator('.ant-select-item-option-content').getByText('model-a', { exact: true }).click()
  await expect(dialog.getByText('公开模型 model-a → 上游模型 upstream-a')).toBeVisible()
  await expect(dialog.getByRole('checkbox', { name: '开启定时监测' })).not.toBeChecked()
  await expect(dialog.getByRole('checkbox', { name: '保存后立即检测' })).toBeChecked()
  await dialog.getByRole('button', { name: '添加服务体检', exact: true }).click()
  await expect(dialog).not.toBeVisible()
  const creation = writes.find(w => w.method === 'POST' && w.path === '/quality-monitors')!
  expect(creation.body).toMatchObject({ channel_id: channel.id, model: 'model-a', run_immediately: true, settings: { schedule_enabled: false, route_policy: 'observe' } })
  const row = page.getByRole('row').filter({ has: page.getByText('model-a', { exact: true }) })
  await expect(row.getByText('仅手动', { exact: true })).toBeVisible()
  await row.getByRole('button', { name: '体检', exact: true }).click()
  await expect(row.getByText('等待检测…')).toBeVisible()
  expect(writes.some(w => w.path.endsWith('/run'))).toBe(true)
  await row.getByRole('button', { name: '删除 测试渠道 / model-a 监控' }).click()
  const confirm = page.getByRole('alertdialog')
  await expect(confirm).toContainText('历史记录保留')
  await confirm.getByRole('button', { name: '确认', exact: true }).click()
  await expect(page.getByRole('heading', { name: '还没有服务体检' })).toBeVisible()
})

test('edit scheduling without changing model identity and verify desktop/mobile layout', async ({ page }) => {
  const writes = await setup(page, true)
  await page.setViewportSize({ width: 1440, height: 1000 })
  await page.getByRole('button', { name: '编辑', exact: true }).click()
  const dialog = page.getByRole('dialog', { name: '编辑服务体检' })
  await expect(dialog.getByText('渠道和模型固定，更换检测对象请新建监控。')).toBeVisible()
  await dialog.getByRole('checkbox', { name: '开启定时监测' }).check()
  await expect(dialog.getByLabel('检测频率')).toBeVisible()
  await dialog.getByLabel('检测频率').click()
  await page.getByRole('option', { name: '15 分钟', exact: true }).click()
  await dialog.getByRole('button', { name: '保存设置' }).click()
  expect(writes.find(w => w.method === 'PATCH')?.body).toMatchObject({ schedule_enabled: true, interval_seconds: 900 })
  await expect(page.getByText('每 15 分钟')).toBeVisible()
  await page.screenshot({ path: '/tmp/vair-quality-desktop.png', fullPage: true })
  await page.getByRole('button', { name: '编辑', exact: true }).click()
  await dialog.getByRole('checkbox', { name: '开启定时监测' }).uncheck()
  await expect(dialog.getByLabel('检测频率')).not.toBeVisible()
  await dialog.getByRole('button', { name: '保存设置' }).click()
  await expect(dialog).not.toBeVisible()
  await page.setViewportSize({ width: 390, height: 844 })
  await page.goto('/monitoring?tab=quality')
  await expect(page.getByRole('heading', { name: '服务体检', exact: true })).toBeVisible()
  await page.screenshot({ path: '/tmp/vair-quality-mobile.png', fullPage: false })
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
  await page.getByRole('button', { name: '添加服务体检', exact: true }).click()
  const heading = page.getByRole('heading', { name: '添加服务体检' })
  await expect(heading).toBeVisible()
  expect(await heading.evaluate(el => { const box = el.getBoundingClientRect(); return el.contains(document.elementFromPoint(box.x + 2, box.y + 2)) })).toBe(true)
  await page.screenshot({ path: '/tmp/vair-quality-editor-mobile.png', fullPage: false })
})

test('monitor details open from the list and keep their address', async ({ page }) => {
  await setup(page, true)
  await page.getByRole('button', { name: '详情', exact: true }).click()
  await expect(page.getByRole('dialog', { name: '服务体检详情' })).toContainText('测试渠道 / model-a')
  await expect(page).toHaveURL(/monitor=22222222/)
})

test('prefix channels suggest matched concrete models, exact rules, aliases and allowed discoveries', async ({ page }, testInfo) => {
  const scoped = { ...channel, models: ['model-*', 'private-exact'], known_models: ['model-a', 'model-b'], discovered_models: ['model-new', 'unrelated', 'upstream-a'], test_model: 'probe-only', model_mapping: { 'model-a': 'upstream-a', 'team-alias': 'upstream-private' } }
  const writes = await setup(page, false, [scoped])
  await page.getByRole('button', { name: '添加服务体检', exact: true }).first().click()
  const dialog = page.getByRole('dialog', { name: '添加服务体检' })
  const input = dialog.getByLabel('模型', { exact: true })
  await expect(input).toBeDisabled()
  await dialog.getByLabel('渠道', { exact: true }).click()
  await page.getByRole('option', { name: '测试渠道', exact: true }).click()
  await input.click()
  const options = page.locator('.ant-select-item-option-content')
  await expect(options).toHaveText(['model-a', 'model-b', 'model-new', 'private-exact', 'probe-only', 'team-alias'])
  await page.screenshot({ path: testInfo.outputPath('quality-model-options-desktop.png'), fullPage: true, animations: 'disabled' })
  await input.fill('team')
  await expect(options).toHaveText(['team-alias'])
  await input.press('ArrowDown')
  await input.press('Enter')
  await expect(input).toHaveValue('team-alias')
  await expect(dialog).toBeVisible()
  await expect(dialog.getByText('公开模型 team-alias → 上游模型 upstream-private')).toBeVisible()
  await dialog.getByRole('button', { name: '添加服务体检', exact: true }).click()
  await expect(dialog).not.toBeVisible()
  expect(writes.find(w => w.path === '/quality-monitors')?.body.model).toBe('team-alias')
})

test('switching to passthrough replaces suggestions and still permits manual concrete model entry on mobile', async ({ page }, testInfo) => {
  const passthrough = { ...channel, id: '33333333-3333-4333-8333-333333333333', name: '透传渠道', protocol: 'openai', models: [], known_models: ['gpt-known'], discovered_models: ['gpt-new'], model_mapping: {}, test_model: 'gpt-probe', model_mode: 'passthrough' }
  const writes = await setup(page, false, [channel, passthrough])
  await page.setViewportSize({ width: 390, height: 844 })
  await page.getByRole('button', { name: '添加服务体检', exact: true }).first().click()
  const dialog = page.getByRole('dialog', { name: '添加服务体检' })
  await dialog.getByLabel('渠道', { exact: true }).click()
  await page.getByRole('option', { name: '测试渠道', exact: true }).click()
  const input = dialog.getByLabel('模型', { exact: true })
  await input.fill('model-a')
  await dialog.getByLabel('渠道', { exact: true }).click()
  await page.getByRole('option', { name: '透传渠道', exact: true }).click()
  await expect(input).toHaveValue('')
  await input.click()
  await expect(page.locator('.ant-select-item-option-content')).toHaveText(['gpt-known', 'gpt-new', 'gpt-probe'])
  await page.screenshot({ path: testInfo.outputPath('quality-model-options-mobile.png'), fullPage: true, animations: 'disabled' })
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
  await input.fill('gpt-private-unlisted')
  await dialog.getByRole('button', { name: '添加服务体检', exact: true }).click()
  await expect(dialog).not.toBeVisible()
  expect(writes.find(w => w.path === '/quality-monitors')?.body).toMatchObject({ channel_id: passthrough.id, model: 'gpt-private-unlisted' })
})

test('non-critical service-check failures show as warnings on a healthy monitor', async ({ page }) => {
  const lastTests = [
    { id: 'integrity', passed: true, critical: true, error_code: null, detail: { extra_chars: 0, extra_text: false } },
    { id: 'replay', passed: true, critical: true, error_code: null, detail: { replayed: false } },
    { id: 'metering', passed: false, critical: false, error_code: null, detail: { tokens_per_char: 0.151, fixed_overhead: 3572, inflated: false, hidden_prompt: true } },
    { id: 'tools', passed: true, critical: false, error_code: null, detail: { tool_called: true } },
  ]
  await setup(page, true, [channel], [{ ...monitor, score: 75, last_tests: lastTests }])
  // Results count passed checks rather than a score.
  await expect(page.getByRole('row').filter({ hasText: 'model-a' })).toContainText('3/4 项通过')
  await expect(page.getByText('75 分')).toHaveCount(0)
  // The status stays plain; details open from the warning icon.
  await expect(page.getByText('疑似隐藏系统提示词（约 3572 Token/请求）')).toBeHidden()
  await page.getByRole('button', { name: '1 项警告' }).click()
  await expect(page.getByRole('dialog')).toContainText('疑似隐藏系统提示词（约 3572 Token/请求）')
})

test('authenticity checks live in their own tab as separate monitors', async ({ page }) => {
  const writes = await setup(page, true)
  await page.getByRole('tab', { name: /真伪检测/ }).click()
  await expect(page).toHaveURL(/tab=identity/)
  await expect(page.getByRole('heading', { name: '还没有真伪检测' })).toBeVisible()
  await expect(page.getByRole('heading', { name: '真伪检测记录' })).toBeVisible()
  await page.getByRole('button', { name: '添加真伪检测' }).first().click()
  const dialog = page.getByRole('dialog', { name: '添加真伪检测' })
  // The add form shows the same settings as editing before a model is chosen.
  await expect(dialog.getByText('选择渠道和模型后自动匹配。')).toBeVisible()
  await expect(dialog.getByRole('checkbox', { name: '开启定时抽样' })).toBeDisabled()
  await expect(dialog.getByLabel('路由影响')).toBeVisible()
  await dialog.getByRole('combobox', { name: '渠道' }).click()
  await page.getByRole('option', { name: '测试渠道' }).click()
  const model = dialog.getByLabel('模型', { exact: true })
  await model.fill('model-a')
  await model.press('ArrowDown')
  await model.press('Enter')
  await expect(model).toHaveValue('model-a')
  // Adding takes the same settings as editing, like a service check.
  await expect(dialog).toContainText('系统还没有启用的模型基准')
  await expect(dialog.getByRole('checkbox', { name: '开启定时抽样' })).toBeDisabled()
  await dialog.getByRole('button', { name: '添加真伪检测', exact: true }).click()
  await expect(dialog).not.toBeVisible()
  expect(writes.find(w => w.method === 'POST' && w.path === '/quality-monitors')?.body).toMatchObject({ model: 'model-a', kind: 'authenticity', authenticity: { claimed_model: null, schedule_enabled: false, route_policy: 'observe' } })
  await expect(page.getByRole('status')).toContainText('已添加真伪检测')
  const row = page.getByRole('row', { name: /测试渠道/ })
  await expect(row).toHaveCount(1)
  await expect(row).toContainText('未检测')
  await page.getByRole('button', { name: '需关注', exact: true }).click()
  await expect(page.getByRole('row', { name: /测试渠道/ })).toHaveCount(0)
  await page.getByRole('button', { name: '全部', exact: true }).click()
  // The service tab still lists only its own monitor.
  await page.getByRole('tab', { name: /服务体检/ }).click()
  await expect(page.getByRole('row', { name: /测试渠道/ })).toHaveCount(1)
  // Older links point at the combined monitor, now the service one: they open
  // the authenticity monitor of the same channel and model.
  await page.goto(`/monitoring?tab=quality&deep=${monitor.id}`)
  const details = page.getByRole('dialog', { name: '真伪检测详情' })
  await expect(details).toContainText('测试渠道 / model-a')
  // Details only show results; checks are started from their own form.
  await expect(details).toContainText('尚未进行深度检测')
  await expect(details.getByRole('button', { name: '开始检测' })).toHaveCount(0)
  await details.getByRole('button', { name: '发起深度检测' }).click()
  await expect(page.getByRole('dialog', { name: '发起深度检测' })).toContainText('系统还没有启用的模型基准')
  await page.keyboard.press('Escape')
  await expect(details).toBeVisible()
  await page.keyboard.press('Escape')
  await expect(page.getByRole('tab', { name: /真伪检测/ })).toHaveAttribute('aria-selected', 'true')
})

test('authenticity monitors are edited with a claim, scheduled sampling, cost estimate and risk acknowledgement', async ({ page }) => {
  baselineModels = [{ model: 'claude-x', family: 'anthropic', per_sample_cost_usd: 0.02, per_sample_tokens: { input: 250, output: 1200 }, cross_family: null, same_family: null, has_siblings: false, evaluated_at: null }]
  const auth = { ...monitor, id: '33333333-3333-4333-8333-333333333333', kind: 'authenticity', interval_seconds: 21600, claimed_model: 'claude-x', identity_verdict: 'match', identity_checked_at: '2026-10-10T09:00:00Z' }
  const writes = await setup(page, true, [channel], [monitor, auth])
  await page.getByRole('tab', { name: /真伪检测/ }).click()
  const row = page.getByRole('row', { name: /测试渠道/ })
  // The status is a plain answer; where it came from sits behind the info button.
  await expect(row).toContainText('相符')
  await row.getByRole('button', { name: '结论依据说明' }).click()
  await expect(page.getByRole('dialog').filter({ hasText: '依据深度检测' })).toBeVisible()
  await page.keyboard.press('Escape')
  await expect(row).toContainText('claude-x')
  await expect(row).toContainText('仅手动')
  await row.getByRole('button', { name: '编辑', exact: true }).click()
  const dialog = page.getByRole('dialog', { name: '编辑真伪检测' })
  // Channel and model are shown once, with the upstream model they map to.
  await expect(dialog.getByText('上游', { exact: true })).toBeVisible()
  await expect(dialog).toContainText('upstream-a')
  await dialog.getByRole('checkbox', { name: '开启定时抽样' }).check()
  // 30 days × 4 runs × 2 samples × $0.02
  await expect(dialog).toContainText('每月约 $4.80')
  const save = dialog.getByRole('button', { name: '保存设置' })
  await expect(save).toBeDisabled()
  await dialog.getByRole('checkbox', { name: '我已了解以上风险' }).check()
  await dialog.getByLabel('路由影响').click()
  await page.getByRole('option', { name: '确认不符时剔除（疑似时降低优先级）' }).click()
  await save.click()
  expect(writes.find(w => w.method === 'PATCH')?.body).toMatchObject({ claimed_model: 'claude-x', schedule_enabled: true, interval_seconds: 21600, samples_per_run: 2, window_days: 7, route_policy: 'exclude', acknowledge_risk: true })
  baselineModels = []
})

test('authenticity records expand in place to show that run\'s report', async ({ page }) => {
  historyItems = [{ id: 'job-1', monitor_id: '33333333-3333-4333-8333-333333333333', model: 'model-a', channel_id: channel.id, channel_name: channel.name, monitor_deleted: true, source: 'deep', status: 'completed', claimed_model: 'claude-x', verdict: 'suspected', valid_samples: 16, completed_samples: 16, target_samples: 32, stopped_early: true, error_code: null, created_at: '2026-10-10T09:00:00Z', finished_at: '2026-10-10T09:05:00Z',
    evidence: { verdict: 'suspected', claimed: 'claude-x', best_model: 'small-y', reasons: ['样本更像 small-y'], valid_samples: 16, samples: { completed: 16, valid: 16, invalid: {} }, comparisons: [{ model: 'small-y', family: 'other', same_family: false, mean_diff: -0.4, z: -2.33, token_consistent: null }] } }]
  await setup(page, true)
  await page.getByRole('tab', { name: /真伪检测/ }).click()
  const record = page.getByRole('row', { name: /claude-x/ })
  await record.getByRole('button', { name: '展开' }).click()
  // Records of deleted monitors still show their report.
  await expect(page.getByText('样本更像 small-y', { exact: true })).toBeVisible()
  // z = -2.33 is about 99% one-sided.
  await expect(page.getByText('置信度 约 99.0%')).toBeVisible()
  await expect(page.getByRole('dialog')).toHaveCount(0)
  await record.getByRole('button', { name: '收起' }).click()
  await expect(page.getByText('样本更像 small-y', { exact: true })).toHaveCount(0)
  historyItems = []
})

test('authenticity status says what users need: progress, the substitute model and unclear results', async ({ page }) => {
  const base = { ...monitor, kind: 'authenticity', interval_seconds: 21600, claimed_model: 'claude-x', schedule_enabled: true }
  const rows = [
    { ...base, id: 'a1', model: 'model-a', window_verdict: 'insufficient', window_checked_at: '2026-10-10T09:00:00Z', window_valid_until: '2099-01-01T00:00:00Z', window_evidence: { verdict: 'insufficient', valid_samples: 5, reasons: [], window: { days: 7, samples: 5 } } },
    { ...base, id: 'a2', model: 'model-b', window_verdict: 'confirmed', window_checked_at: '2026-10-10T09:00:00Z', window_valid_until: '2099-01-01T00:00:00Z', window_evidence: { verdict: 'confirmed', claimed_model: 'claude-x', best_model: 'small-y', valid_samples: 40, reasons: ['样本更像 small-y'], window: { days: 7, samples: 40 } } },
    { ...base, id: 'a3', model: 'model-c', schedule_enabled: false, identity_verdict: 'ambiguous', identity_checked_at: '2026-10-10T09:00:00Z', identity_evidence: { verdict: 'ambiguous', valid_samples: 32 } },
  ]
  await setup(page, true, [channel], rows)
  await page.getByRole('tab', { name: /真伪检测/ }).click()
  const row = (model: string) => page.getByRole('row').filter({ has: page.getByText(model, { exact: true }) })
  await expect(row('model-a')).toContainText('累积中')
  await expect(row('model-a')).toContainText('已有 5/8 个样本')
  await expect(row('model-b')).toContainText('确认不符')
  await expect(row('model-b')).toContainText('实际为 small-y')
  await expect(row('model-c')).toContainText('无法判断')
  await expect(row('model-c')).toContainText('证据互相矛盾')
  // Still accumulating is not something to act on; unclear and mismatched results are.
  await page.getByRole('button', { name: '需关注', exact: true }).click()
  await expect(row('model-a')).toHaveCount(0)
  await expect(row('model-b')).toHaveCount(1)
  await expect(row('model-c')).toHaveCount(1)
})

test('service checks add truncation and streaming, and real traffic warnings show on monitors', async ({ page }) => {
  passiveRows = [
    { channel_id: channel.id, channel_name: channel.name, upstream_model: 'upstream-a', samples: 120, tokens_per_ascii: 0.3, tokens_per_other: null, overhead: 3500, warnings: [{ kind: 'hidden_prompt' }], messages: ['每个请求固定多出约 3500 Token，疑似被加入隐藏系统提示词'], computed_at: '2026-10-11T09:00:00Z', monitor_id: monitor.id },
    { channel_id: channel.id, channel_name: channel.name, upstream_model: 'model-b', samples: 80, tokens_per_ascii: 1.2, tokens_per_other: null, overhead: 100, warnings: [{ kind: 'inflated_ascii' }], messages: ['英文等 ASCII 文本每字符约 1.2 Token，超过合理上限 0.75，疑似用量虚报'], computed_at: '2026-10-11T09:00:00Z', monitor_id: null },
  ]
  await setup(page, true)
  // A warning on a monitored model sits next to its status.
  const row = page.getByRole('row').filter({ has: page.getByText('model-a', { exact: true }) })
  await expect(row).not.toContainText(' 分')
  await row.getByRole('button', { name: '1 项警告' }).click()
  await expect(page.getByRole('dialog').filter({ hasText: '真实流量' })).toContainText('3500 Token')
  await page.keyboard.press('Escape')
  await page.getByRole('button', { name: '需关注', exact: true }).click()
  await expect(row).toHaveCount(1)
  // A model without a service check is listed with a way to add one.
  await expect(page.getByText('真实流量发现 1 处计量异常')).toBeVisible()
  await page.getByRole('button', { name: '为 测试渠道 / model-b 添加服务体检' }).click()
  const dialog = page.getByRole('dialog', { name: '添加服务体检' })
  await expect(dialog.getByLabel('模型', { exact: true })).toHaveValue('model-b')
  for (const name of ['输出截断', '流式输出']) await expect(dialog.getByRole('checkbox', { name: new RegExp(name) })).toBeChecked()
  passiveRows = []
})

test('long-context checks start from details and appear in the records', async ({ page }) => {
  runItems = [{ id: 'run-1', monitor_id: monitor.id, channel_id: channel.id, kind: 'long_context', model: 'model-a', status: 'degraded', score: null, latency_ms: 9000, ttfb_ms: null, status_code: 200, error_code: null, started_at: '2026-10-11T09:00:00Z', finished_at: '2026-10-11T09:01:00Z',
    details: { requested: 128000, max_passed: 32000, tiers: [{ tokens: 8000, input_tokens: 8100, passed: true, latency_ms: 2000, error_code: null }, { tokens: 32000, input_tokens: 31900, passed: true, latency_ms: 3000, error_code: null }, { tokens: 128000, input_tokens: 32000, passed: false, latency_ms: 4000, error_code: null }] } }]
  const writes = await setup(page, true, [channel], [{ ...monitor, long_context_max: 32000, long_context_checked_at: '2026-10-11T09:01:00Z' }])
  await page.goto(`/monitoring?tab=quality&monitor=${monitor.id}`)
  const details = page.getByRole('dialog', { name: '服务体检详情' })
  await expect(details).toContainText('最大通过 32k')
  await expect(details).toContainText('长上下文 · 最大通过 32k / 检测到 128k')
  await details.getByRole('button', { name: '展开' }).click()
  await expect(details).toContainText('没有找到口令')
  await page.keyboard.press('Escape')
  // Started from the check button's menu in the list, next to the service check.
  await page.getByRole('button', { name: '更多检测 测试渠道 / model-a' }).click()
  await page.getByRole('menuitem', { name: '长上下文检测' }).click()
  const form = page.getByRole('dialog', { name: '长上下文检测' })
  await expect(form).toContainText('按官方价格估算最多 $0.10')
  const start = form.getByRole('button', { name: '开始检测' })
  await expect(start).toBeDisabled()
  await form.getByRole('checkbox', { name: '我已了解以上风险' }).check()
  await form.getByText('128k Token').click()
  await start.click()
  await expect(form).not.toBeVisible()
  expect(writes.find(w => w.path.endsWith('/long-context'))?.body).toEqual({ max_tokens: 128000, acknowledge_risk: true })
  runItems = []
})
