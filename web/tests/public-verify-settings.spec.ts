import { expect, test } from '@playwright/test'

// Public verification exists only on the hosted service; run with VITE_EDITION=cloud.
test.skip(process.env.VITE_EDITION !== 'cloud', 'cloud edition only')

test('super admins set the homepage verification capacity', async ({ page }) => {
  let saved: unknown = null
  await page.route('**/api/**', async route => {
    const request = route.request()
    const path = new URL(request.url()).pathname
    const send = (data: unknown, status = 200) => route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(data) })
    if (path === '/api/me') return send({ user_id: 'fixture-admin', email: 'admin@example.invalid', account_id: 'fixture', is_super_admin: true })
    if (path === '/api/account') return send({ id: 'fixture', name: '验证设置', api_key_prefix: 'fixture' })
    if (path === '/api/platform/public-verify' && request.method() === 'PATCH') {
      saved = request.postDataJSON()
      return send(saved)
    }
    if (path === '/api/platform/public-verify') return send({ workers: 4, concurrency: 2, max_waiting: 20 })
    return send([])
  })
  await page.goto('/platform/public-verify')
  await expect(page.getByLabel('同时检测数')).toHaveValue('4')
  await expect(page.getByLabel('单次检测并发请求')).toHaveValue('2')
  await expect(page.getByLabel('排队上限')).toHaveValue('20')
  await page.getByLabel('同时检测数').fill('8')
  await page.getByLabel('排队上限').fill('40')
  await page.getByRole('button', { name: '保存设置' }).click()
  await expect(page.getByRole('status')).toHaveText('已保存')
  expect(saved).toEqual({ workers: 8, concurrency: 2, max_waiting: 40 })
  await page.screenshot({ path: test.info().outputPath('public-verify.png'), fullPage: true })
})
