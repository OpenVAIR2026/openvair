import { expect, test } from '@playwright/test'
import type { Channel, ModelCatalogEntry } from '../src/lib/api'

const entry = (pattern: string): ModelCatalogEntry => ({ id: pattern, pattern, label: pattern, enabled: true, provider: '', protocols: ['openai'], input_per_1m: null, output_per_1m: null, cached_read_per_1m: null, price_source: null, created_at: '', updated_at: '' })
const channel = (id: string, patch: Partial<Channel> = {}) => ({ id, name: id, protocol: 'openai', base_url: 'https://upstream.example.invalid', enabled: true, priority: 0, health_status: 'healthy', health_error: null, health_error_kind: null, health_note: null, health_checked_at: null, test_model: null, models: [], model_mapping: {}, model_mode: 'passthrough', model_count: 0, known_models: [], discovered_models: [], models_synced_at: null, created_at: '', kind: 'upstream', provider: null, source: null, ...patch })

test('lists each channel\'s uncatalogued models as they arrive and adds the chosen ones', async ({ page }) => {
  let release = () => {}
  const slow = new Promise<void>(resolve => { release = resolve })
  const synced: string[] = []
  const posts: { pattern: string; protocols?: string[] }[] = []
  const catalog = [entry('gpt-6')]
  await page.route('**/api/**', async route => {
    const path = new URL(route.request().url()).pathname
    const method = route.request().method()
    const send = (data: unknown, status = 200) => route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(data) })
    if (path === '/api/me') return send({ user_id: 'fixture-admin', email: 'admin@example.invalid', account_id: 'fixture', is_super_admin: true })
    if (path === '/api/account') return send({ id: 'fixture', name: '发现新模型', api_key_prefix: 'fixture' })
    if (path === '/api/channels') return send([
      channel('fast', { discovered_models: ['gpt-6', 'gpt-6.1-sol'], models_synced_at: '2026-10-01T00:00:00Z' }),
      channel('slow', { protocol: 'anthropic' }),
      channel('broken'),
      channel('off', { enabled: false }),
      channel('vair', { kind: 'vair_key', protocol: 'any' }),
    ])
    const sync = path.match(/^\/api\/channels\/([^/]+)\/sync-models$/)
    if (sync) {
      synced.push(sync[1])
      if (sync[1] === 'fast') return send({ models: ['gpt-6', 'gpt-6.1-sol', 'gpt-6.1-terra', 'gpt-*'] })
      if (sync[1] === 'slow') { await slow; return send({ models: ['gpt-6.1-terra', 'claude-next'] }) }
      return send({ error: { message: '上游拒绝了请求' } }, 502)
    }
    if (path === '/api/model-catalog') {
      if (method === 'POST') {
        const input = route.request().postDataJSON()
        posts.push(input)
        const model = { ...entry(input.pattern), ...input }
        catalog.push(model)
        return send(model)
      }
      return send(catalog)
    }
    return send([])
  })
  await page.goto('/platform/model-catalog')
  await page.getByRole('button', { name: '发现新模型' }).click()
  const dialog = page.getByRole('dialog', { name: '发现新模型' })
  await expect(dialog.getByRole('checkbox', { name: '选择加入 gpt-6.1-terra' })).toBeVisible()
  await expect(dialog.getByRole('checkbox', { name: '选择加入 gpt-6.1-sol' })).toBeVisible()
  await expect(dialog.getByText('上次同步后新增')).toHaveCount(1)
  await expect(dialog.getByRole('checkbox', { name: '选择加入 claude-next' })).toHaveCount(0)
  await expect(dialog.getByText('broken · 失败')).toBeVisible()
  await expect(dialog.getByRole('status').first()).toContainText('正在检查渠道 2/3')
  release()
  await expect(dialog.getByRole('checkbox', { name: '选择加入 claude-next' })).toBeVisible()
  await expect(dialog.getByRole('status').first()).toContainText('已检查 3 个渠道')
  expect(synced.sort()).toEqual(['broken', 'fast', 'slow'])
  await expect(dialog.getByRole('checkbox', { name: /选择加入 gpt-6$|选择加入 gpt-\*/ })).toHaveCount(0)
  const slowTag = dialog.getByRole('button', { name: 'slow · 2' })
  await slowTag.click()
  await expect(slowTag).toHaveAttribute('aria-pressed', 'true')
  await expect(dialog.getByRole('checkbox', { name: '选择加入 gpt-6.1-sol' })).toHaveCount(0)
  await expect(dialog.getByRole('checkbox', { name: '选择加入 claude-next' })).toBeVisible()
  await expect(dialog.getByText('slow：2 / 3 个')).toBeVisible()
  await expect(dialog.getByRole('button', { name: /^broken/ })).toHaveCount(0)
  await slowTag.click()
  await expect(dialog.getByRole('checkbox', { name: '选择加入 gpt-6.1-sol' })).toBeVisible()
  await dialog.getByRole('checkbox', { name: '选择加入 gpt-6.1-terra' }).click()
  await dialog.getByRole('button', { name: '加入所选（1）' }).click()
  await expect(dialog.getByText('已加入 1 个模型。')).toBeVisible()
  expect(posts).toEqual([{ pattern: 'gpt-6.1-terra', label: 'gpt-6.1-terra', enabled: true, protocols: ['openai', 'anthropic'] }])
  await expect(dialog.getByRole('checkbox', { name: '选择加入 gpt-6.1-terra' })).toHaveCount(0)
})
