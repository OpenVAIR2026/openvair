import { expect, test, type Page } from '@playwright/test'
import type { ModelCatalogEntry, ModelPrice } from '../src/lib/api'

const entry = (pattern: string): ModelCatalogEntry => ({ id: pattern, pattern, label: pattern, enabled: true, provider: '', protocols: ['openai', 'anthropic', 'gemini'], input_per_1m: null, output_per_1m: null, cached_read_per_1m: null, price_source: null, created_at: '', updated_at: '' })
const reference = (pattern: string, label = pattern): ModelPrice => ({ pattern, label, input_per_1m: 2, output_per_1m: 8, cached_read_per_1m: 0.5, synced_from: 'openrouter' })

async function fixtures(page: Page) {
  const state = {
    catalog: [{ ...entry('gpt-4.1'), input_per_1m: 7 }, { ...entry('gpt-hidden'), enabled: false }, entry('private-only')],
    reference: [reference('gpt-4.1'), reference('gpt-4.1-mini', 'GPT 4.1 Mini'), reference('gpt-4.1-nano'), reference('gpt-hidden'), reference('claude-sonnet-4', 'Claude Sonnet 四代'), reference('gpt-*')],
    posts: [] as { pattern: string; label: string; enabled: boolean; price?: unknown }[],
    writes: [] as string[],
    failures: new Set<string>(),
    failCatalog: false,
    failReference: false,
    referenceGate: null as Promise<void> | null,
    saveGate: null as Promise<void> | null,
  }
  await page.route('**/api/**', async route => {
    const path = new URL(route.request().url()).pathname
    const method = route.request().method()
    const send = (data: unknown, status = 200) => route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(data) })
    if (method !== 'GET') state.writes.push(`${method} ${path}`)
    if (path === '/api/me') return send({ user_id: 'fixture-admin', email: 'admin@example.invalid', account_id: 'fixture', is_super_admin: true })
    if (path === '/api/account') return send({ id: 'fixture', name: '模型选择验证', api_key_prefix: 'fixture' })
    if (path === '/api/model-catalog') {
      if (method === 'GET') return state.failCatalog ? send({ error: { message: '目录暂不可用' } }, 503) : send(state.catalog)
      if (method === 'POST') {
        const input = route.request().postDataJSON()
        state.posts.push(input)
        if (state.saveGate) await state.saveGate
        if (state.catalog.some(model => model.pattern === input.pattern)) return send({ error: { message: 'model pattern already exists' } }, 409)
        if (state.failures.has(input.pattern)) return send({ error: { message: '模拟添加失败' } }, 503)
        const model = { ...entry(input.pattern), ...input }
        state.catalog.push(model)
        return send(model)
      }
    }
    if (path === '/api/platform/model-pricing/catalog') {
      if (state.referenceGate) await state.referenceGate
      return state.failReference ? send({ error: { message: '参考目录暂不可用' } }, 503) : send(state.reference)
    }
    return send([])
  })
  return state
}

async function openPicker(page: Page) {
  await page.goto('/platform/model-catalog')
  await page.getByRole('button', { name: '添加模型', exact: true }).click()
  return page.getByRole('dialog', { name: '添加模型', exact: true })
}

test('fuzzy search lists exact membership and select-all stays within current results', async ({ page }) => {
  const state = await fixtures(page)
  state.reference.push(reference('gpt-4.1-mini'), reference('GPT-4.1'))
  const dialog = await openPicker(page)
  const search = dialog.getByRole('searchbox', { name: '搜索模型' })
  await expect(search).toBeFocused()
  await expect(dialog.getByRole('checkbox', { name: '已加入 gpt-4.1', exact: true })).toBeDisabled()
  await expect(dialog.getByRole('checkbox', { name: '已加入 gpt-hidden', exact: true })).toBeDisabled()
  await expect(dialog.getByRole('checkbox', { name: '已加入 private-only', exact: true })).toBeDisabled()
  await expect(dialog.getByRole('checkbox', { name: '选择添加 GPT-4.1', exact: true })).toBeEnabled()
  await expect(dialog.getByRole('checkbox', { name: '选择添加 gpt-4.1-mini', exact: true })).toHaveCount(1)
  await expect(dialog.getByRole('checkbox', { name: /gpt-\*/ })).toHaveCount(0)
  await search.fill(' GPT41 MINI ')
  await expect(dialog.getByRole('listitem')).toHaveCount(1)
  await dialog.getByRole('checkbox', { name: '勾选全部未加入模型' }).check()
  await search.fill('四代 SONNET')
  await expect(dialog.getByRole('listitem')).toHaveCount(1)
  await expect(dialog.getByText('（含其他搜索结果 1 个）')).toBeVisible()
  await dialog.getByRole('checkbox', { name: '选择添加 claude-sonnet-4' }).focus()
  await page.keyboard.press('Space')
  await search.fill('gpt')
  const all = dialog.getByRole('checkbox', { name: '勾选全部未加入模型' })
  await expect(all).toHaveAttribute('data-state', 'indeterminate')
  await all.check()
  await expect(dialog.getByRole('button', { name: '添加所选（4）' })).toBeEnabled()
  await all.uncheck()
  await expect(dialog.getByRole('button', { name: '添加所选（1）' })).toBeEnabled()
  await search.fill('private-only')
  await expect(all).toBeDisabled()
  await search.fill('not-in-catalog')
  await expect(dialog.getByText('未找到匹配模型')).toBeVisible()
  await dialog.getByRole('button', { name: '清空搜索' }).click()
  await expect(dialog.getByRole('checkbox', { name: '选择添加 claude-sonnet-4' })).toBeChecked()
  await dialog.getByRole('button', { name: '清空选择' }).click()
  await expect(dialog.getByRole('button', { name: '添加所选（0）' })).toBeDisabled()
  expect(state.posts).toHaveLength(0)
})

test('batch add keeps existing settings and only retries failed models', async ({ page }) => {
  const state = await fixtures(page)
  const original = structuredClone(state.catalog)
  state.failures.add('gpt-4.1-mini')
  const dialog = await openPicker(page)
  await dialog.getByRole('checkbox', { name: '勾选全部未加入模型' }).check()
  await dialog.getByRole('button', { name: '添加所选（3）' }).click()
  await expect(dialog.getByRole('alert')).toContainText('1 个添加失败')
  await expect(dialog.getByRole('checkbox', { name: '选择添加 gpt-4.1-mini' })).toBeChecked()
  await expect(dialog.getByRole('checkbox', { name: '已加入 gpt-4.1-nano' })).toBeDisabled()
  await expect(dialog.getByRole('checkbox', { name: '已加入 claude-sonnet-4' })).toBeDisabled()
  expect(state.posts).toHaveLength(3)
  expect(state.posts.every(post => post.price === undefined)).toBe(true)
  expect(state.catalog.slice(0, original.length)).toEqual(original)
  state.failures.clear()
  await dialog.getByRole('button', { name: '添加所选（1）' }).click()
  await expect(dialog.getByRole('checkbox', { name: '已加入 gpt-4.1-mini' })).toBeDisabled()
  await expect(dialog.getByRole('button', { name: '添加所选（0）' })).toBeDisabled()
  expect(state.posts.map(post => post.pattern).filter(pattern => pattern === 'gpt-4.1-mini')).toHaveLength(2)
  expect(state.posts).toHaveLength(4)
  expect(state.writes.every(write => write === 'POST /api/model-catalog')).toBe(true)
  await dialog.getByRole('button', { name: '完成', exact: true }).click()
  await expect(page.getByRole('button', { name: '编辑 gpt-4.1-mini', exact: true })).toBeVisible()
})

test('fresh membership checks skip additions made by another administrator', async ({ page }) => {
  const state = await fixtures(page)
  const dialog = await openPicker(page)
  await dialog.getByRole('checkbox', { name: '选择添加 gpt-4.1-mini' }).check()
  state.catalog.push({ ...entry('gpt-4.1-mini'), enabled: false, input_per_1m: 23, label: '另一个管理员添加' })
  await dialog.getByRole('button', { name: '添加所选（1）' }).click()
  await expect(dialog.getByText('已加入 0 个模型。 1 个模型已在目录中，已跳过。')).toBeVisible()
  await expect(dialog.getByRole('checkbox', { name: '已加入 gpt-4.1-mini' })).toBeDisabled()
  expect(state.posts).toHaveLength(0)
  expect(state.catalog.at(-1)).toMatchObject({ enabled: false, input_per_1m: 23, label: '另一个管理员添加' })
})

test('submission locks dismissal and concurrent duplicate creates are skipped', async ({ page }) => {
  const state = await fixtures(page)
  let release!: () => void
  state.saveGate = new Promise<void>(resolve => { release = resolve })
  const dialog = await openPicker(page)
  await dialog.getByRole('checkbox', { name: '选择添加 gpt-4.1-mini' }).check()
  await dialog.getByRole('button', { name: '添加所选（1）' }).click()
  await expect.poll(() => state.posts.length).toBe(1)
  await expect(dialog.getByRole('button', { name: '关闭添加模型', exact: true })).toBeDisabled()
  await expect(dialog.getByRole('button', { name: '手动添加', exact: true })).toBeDisabled()
  await page.keyboard.press('Escape')
  await expect(dialog).toBeVisible()
  state.catalog.push({ ...entry('gpt-4.1-mini'), input_per_1m: 23 })
  release()
  await expect(dialog.getByText('已加入 0 个模型。 1 个模型已在目录中，已跳过。')).toBeVisible()
  await expect(dialog.getByRole('checkbox', { name: '已加入 gpt-4.1-mini' })).toBeDisabled()
  expect(state.catalog.at(-1)?.input_per_1m).toBe(23)
})

for (const source of ['catalog', 'reference'] as const) {
  test(`${source} loading errors prevent selection and recover on retry`, async ({ page }) => {
    const state = await fixtures(page)
    state.failCatalog = source === 'catalog'
    state.failReference = source === 'reference'
    const dialog = await openPicker(page)
    await expect(dialog.getByRole('alert')).toContainText('模型列表加载失败')
    await expect(dialog.getByRole('checkbox', { name: '勾选全部未加入模型' })).toBeDisabled()
    await expect(dialog.getByRole('button', { name: '添加所选（0）' })).toBeDisabled()
    state.failCatalog = false
    state.failReference = false
    await dialog.getByRole('button', { name: '重试' }).click()
    await expect(dialog.getByRole('checkbox', { name: '勾选全部未加入模型' })).toBeEnabled()
    expect(state.posts).toHaveLength(0)
  })
}

test('initial loading and preflight failures retain safe selection behavior', async ({ page }) => {
  const state = await fixtures(page)
  let release!: () => void
  state.referenceGate = new Promise<void>(resolve => { release = resolve })
  const dialog = await openPicker(page)
  await expect(dialog.getByText('正在获取模型…')).toBeVisible()
  await expect(dialog.getByRole('checkbox', { name: '勾选全部未加入模型' })).toBeDisabled()
  release()
  await dialog.getByRole('checkbox', { name: '选择添加 gpt-4.1-mini' }).check()
  state.failCatalog = true
  await dialog.getByRole('button', { name: '添加所选（1）' }).click()
  await expect(dialog.getByRole('alert').filter({ hasText: '目录暂不可用' })).toBeVisible()
  expect(state.posts).toHaveLength(0)
  state.failCatalog = false
  await dialog.getByRole('button', { name: '重试' }).click()
  await expect(dialog.getByRole('checkbox', { name: '选择添加 gpt-4.1-mini' })).toBeChecked()
  await dialog.getByRole('button', { name: '添加所选（1）' }).click()
  await expect(dialog.getByRole('checkbox', { name: '已加入 gpt-4.1-mini' })).toBeDisabled()
})

test('empty sources offer manual addition without fabricated candidates', async ({ page }) => {
  const state = await fixtures(page)
  state.catalog = []
  state.reference = []
  const dialog = await openPicker(page)
  await expect(dialog.getByText('暂无候选模型')).toBeVisible()
  await expect(dialog.getByRole('button', { name: '添加所选（0）' })).toBeDisabled()
  await dialog.getByRole('button', { name: '手动添加' }).click()
  await expect(page.getByLabel('模型 ID', { exact: true })).toBeVisible()
  expect(state.posts).toHaveLength(0)
})

test('desktop and narrow mobile keep selection actions visible while the list scrolls', async ({ page }, testInfo) => {
  const state = await fixtures(page)
  state.reference.push(...Array.from({ length: 40 }, (_, i) => reference(`vendor/model-with-a-long-context-name-and-version-${i}`, `模型显示名称 ${i}`)))
  const dialog = await openPicker(page)
  await dialog.getByRole('checkbox', { name: '选择添加 claude-sonnet-4' }).check()
  await page.screenshot({ path: testInfo.outputPath('model-add-desktop.png') })
  await page.setViewportSize({ width: 390, height: 844 })
  await dialog.getByRole('searchbox', { name: '搜索模型' }).fill('gpt')
  await dialog.getByRole('checkbox', { name: '勾选全部未加入模型' }).check()
  await page.screenshot({ path: testInfo.outputPath('model-add-mobile.png') })
  await page.setViewportSize({ width: 320, height: 640 })
  await dialog.getByRole('searchbox', { name: '搜索模型' }).fill('')
  const list = dialog.getByRole('region', { name: '可添加模型列表' })
  await list.evaluate(element => { element.scrollTop = element.scrollHeight })
  const action = dialog.getByRole('button', { name: '添加所选（3）' })
  await expect(action).toBeInViewport()
  expect(await action.evaluate(element => { const box = element.getBoundingClientRect(); return element.contains(document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2)) })).toBe(true)
  expect(await dialog.evaluate(element => element.scrollWidth <= element.clientWidth)).toBe(true)
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
  await page.screenshot({ path: testInfo.outputPath('model-add-mobile-narrow.png') })
})
