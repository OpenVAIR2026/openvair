import { expect, test, type Page } from '@playwright/test'
import type { BaselineBatch, BaselineModel, BaselineVersion } from '../src/lib/authenticity'

const progress = { batches: 2, token_target: 9, token_counted: 9, token_missing: 0 }
const summary = { id: 'v2', status: 'active' as const, revision: 2, valid_samples: 200, pool_version: 1, created_at: '2026-10-10T11:58:30Z', activated_at: '2026-10-10T12:10:00Z', evaluation: null, evaluated_revision: 2, evaluated_at: '2026-10-10T12:10:00Z', progress }
const model: BaselineModel = { model: 'gpt-6-astra', label: null, family: 'openai', openrouter_model: 'openai/gpt-6-astra', provider_only: 'openai', target_samples: 200, token_samples: 9, reasoning: 'default', active: summary, draft: null, pending_batches: 1 }
const manifest = { openrouter_model: 'openai/gpt-6-astra', provider_only: 'openai', pool_version: 1, tool_version: 'authenticity_spike/2', started_at: '', finished_at: '' }
const batch = (b: Partial<BaselineBatch>): BaselineBatch => ({ id: 'b', model: 'gpt-6-astra', version_id: 'v2', status: 'merged', manifest, validation: { samples: 1, duplicates: 0, valid: 1, fingerprint_valid: 1, token_valid: 0, invalid: {}, compatible_version_id: 'v2' }, drift: null, preview: null, sample_count: 1, valid_count: 1, counts: {}, version: { revision: 2, status: 'active', valid_samples: 200 }, compatible_revision: 2, target_samples: 200, created_at: '2026-10-10T12:03:31Z', decided_at: null, excluded_at: null, ...b })
const imports = [
  batch({ id: 'drifted', status: 'pending', version_id: null, version: null, drift: { checked: true, jsd: 0.2, p_value: 0.001, flagged: true }, counts: { fingerprint: { total: 40, valid: 40, invalid: {} } } }),
  batch({ id: 'topup', counts: { fingerprint: { total: 1, valid: 1, invalid: {} } } }),
  batch({ id: 'first', validation: { samples: 209, duplicates: 0, valid: 208, fingerprint_valid: 199, token_valid: 9, invalid: { provider_mismatch: 1 }, compatible_version_id: null }, compatible_revision: null, sample_count: 209, valid_count: 208, created_at: '2026-10-10T11:58:18Z', counts: { fingerprint: { total: 200, valid: 199, invalid: { no_provider: 1 } }, token: { total: 9, valid: 9, invalid: {} } } }),
]
const versions: BaselineVersion[] = [{ ...summary, provider_only: 'openai', batches: [
  { id: 'first', valid_count: 208, sample_count: 209, counts: imports[2].counts, created_at: imports[2].created_at, excluded_at: null },
  { id: 'topup', valid_count: 1, sample_count: 1, counts: imports[1].counts, created_at: imports[1].created_at, excluded_at: null },
] }]

async function fixtures(page: Page) {
  await page.route('**/api/**', async route => {
    const path = new URL(route.request().url()).pathname
    const send = (data: unknown) => route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(data) })
    if (path === '/api/me') return send({ user_id: 'fixture-admin', email: 'admin@example.invalid', account_id: 'fixture', is_super_admin: true })
    if (path === '/api/account') return send({ id: 'fixture', name: '基准验证', api_key_prefix: 'fixture' })
    if (path === '/api/platform/model-baselines') return send({ models: [model], pool_version: 1, range_max: 355 })
    if (path === '/api/platform/model-baselines/imports') return send(imports)
    if (path.endsWith('/versions')) return send(versions)
    return send([])
  })
}

test('imports separate fingerprint and Tokenizer samples and say where each batch went', async ({ page }) => {
  await fixtures(page)
  await page.goto('/platform/model-baselines')
  await page.getByRole('tab', { name: '导入记录' }).click()
  const rows = page.locator('tbody tr')
  await expect(rows.nth(0)).toContainText('只能作为新版本')
  await expect(rows.nth(1)).toContainText('池 v1 · 增量补采')
  await expect(rows.nth(1)).toContainText('修订 2（启用中）当前指纹 200/200')
  await expect(rows.nth(2)).toContainText('池 v1 · 新版本')
  await expect(rows.nth(2)).toContainText('199/2001 个未返回供应商')
  await expect(rows.nth(2)).toContainText('9/9')
  await page.screenshot({ path: test.info().outputPath('imports.png'), fullPage: true })
})

test('models and versions show fingerprint and Tokenizer progress', async ({ page }) => {
  await fixtures(page)
  await page.goto('/platform/model-baselines')
  await expect(page.locator('tbody tr').first()).toContainText('指纹样本 200/200 · Tokenizer 9/9')
  await expect(page.getByText('待补采', { exact: false })).toHaveCount(0)
  await page.getByRole('button', { name: '版本' }).click()
  const sheet = page.getByRole('dialog')
  await expect(sheet).toContainText('指纹样本 200/200 · Tokenizer 9/9 · 2 个批次')
  await expect(sheet).toContainText('指纹 199/200 有效 · Tokenizer 9/9 有效')
  await expect(sheet).toContainText('指纹 1/1 有效排除')
  await page.screenshot({ path: test.info().outputPath('versions.png'), fullPage: true })
})
