import { expect, test, type Page } from '@playwright/test'

const chrome = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36'
const iphone = 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1'

async function fixtures(page: Page) {
  const state = {
    email: 'owner@example.invalid',
    account: { id: 'account', name: '安全验证', api_key_prefix: 'vair', channel_limit: 10, virtual_key_limit: 10, alert_emails: { authenticity: true, service: true, metering: true } },
    sessions: [
      { id: 'current', created_at: '2026-10-01T08:00:00Z', last_seen_at: '2026-10-07T08:00:00Z', expires_at: '2026-10-08T08:00:00Z', user_agent: chrome, current: true },
      { id: 'phone', created_at: '2026-10-02T08:00:00Z', last_seen_at: null, expires_at: '2026-10-09T08:00:00Z', user_agent: iphone, current: false },
      { id: 'legacy', created_at: '2026-10-03T08:00:00Z', last_seen_at: null, expires_at: '2026-10-10T08:00:00Z', user_agent: null, current: false },
    ],
    writes: [] as { method: string; path: string; body: unknown }[],
    failures: {} as Record<string, { status: number; message: string }>,
  }
  await page.route('**/api/**', async route => {
    const path = new URL(route.request().url()).pathname
    const method = route.request().method()
    const send = (body: unknown, status = 200) => route.fulfill({ status, contentType: 'application/json', body: status === 204 ? undefined : JSON.stringify(body) })
    if (method !== 'GET') {
      const body = route.request().postData() ? route.request().postDataJSON() : null
      state.writes.push({ method, path, body })
      const failure = state.failures[path]
      if (failure) return send({ error: { message: failure.message } }, failure.status)
      if (path === '/api/me' && method === 'PATCH') state.account.name = body.account_name
      if (path === '/api/account' && method === 'PATCH') Object.assign(state.account, body)
      if (path === '/api/me/email-verification') return send({ request_id: 'request-1', expires_in: 600, retry_after: 60 })
      if (path === '/api/me/email-verification/confirm') { state.email = body.email; return send({}) }
      if (path === '/api/me/sessions/revoke-others') { const revoked = state.sessions.filter(s => !s.current).length; state.sessions = state.sessions.filter(s => s.current); return send({ revoked }) }
      if (path.startsWith('/api/me/sessions/') && method === 'DELETE') { state.sessions = state.sessions.filter(s => s.id !== path.split('/').at(-1)); return send(null, 204) }
      return path === '/api/me/password' ? send(null, 204) : send({})
    }
    if (path === '/api/me') return send({ user_id: 'user', email: state.email, account_id: 'account', is_super_admin: false })
    if (path === '/api/account') return send(state.account)
    if (path === '/api/me/sessions') return send(state.sessions)
    return send([])
  })
  return state
}

const fillPassword = async (page: Page, current: string, next: string, confirm = next) => {
  await page.getByLabel('当前密码', { exact: true }).fill(current)
  await page.getByLabel('新密码', { exact: true }).fill(next)
  await page.getByLabel('确认新密码', { exact: true }).fill(confirm)
}

test('password changes are validated before any request', async ({ page }) => {
  const state = await fixtures(page)
  await page.goto('/settings')
  await page.getByRole('button', { name: '修改登录密码' }).click()
  for (const [current, next, confirm, message] of [
    ['', 'new-password-value', 'new-password-value', '请输入当前密码'],
    ['old-password-value', 'short', 'short', '新密码长度需为 10–256 个字符'],
    ['old-password-value', 'new-password-value', 'other-password-value', '两次输入的新密码不一致'],
    ['old-password-value', 'old-password-value', 'old-password-value', '新密码不能与当前密码相同'],
  ]) {
    await fillPassword(page, current, next, confirm)
    await page.getByRole('button', { name: '更新密码', exact: true }).click()
    await expect(page.getByRole('alert')).toHaveText(message)
  }
  expect(state.writes).toHaveLength(0)
})

test('a successful password change closes the editor; server errors keep the input', async ({ page }) => {
  const state = await fixtures(page)
  await page.goto('/settings')
  await page.getByRole('button', { name: '修改登录密码' }).click()
  state.failures['/api/me/password'] = { status: 400, message: '当前密码不正确' }
  await fillPassword(page, 'wrong-password-value', 'new-password-value')
  await page.getByLabel('确认新密码', { exact: true }).press('Enter')
  await expect(page.getByText('当前密码不正确')).toBeVisible()
  await expect(page.getByLabel('新密码', { exact: true })).toHaveValue('new-password-value')
  delete state.failures['/api/me/password']
  await page.getByLabel('当前密码', { exact: true }).fill('old-password-value')
  await page.getByLabel('确认新密码', { exact: true }).press('Enter')
  await expect(page.getByText('密码已更新，其他设备上的登录已退出。')).toBeVisible()
  await expect(page.getByLabel('新密码', { exact: true })).toHaveCount(0)
  expect(state.writes.at(-1)).toEqual({ method: 'POST', path: '/api/me/password', body: { current_password: 'old-password-value', new_password: 'new-password-value' } })
  expect(state.writes.every(write => write.path === '/api/me/password')).toBe(true)
})

test('account settings save one field at a time', async ({ page }) => {
  const state = await fixtures(page)
  await page.goto('/settings')
  await page.getByRole('button', { name: '修改帐号名称' }).click()
  await page.getByLabel('帐号名称', { exact: true }).fill('新的帐号名称')
  await page.getByLabel('帐号名称', { exact: true }).press('Enter')
  await expect(page.locator('.settings-page').getByText('新的帐号名称', { exact: true })).toBeVisible()
  expect(state.writes).toEqual([{ method: 'PATCH', path: '/api/me', body: { account_name: '新的帐号名称' } }])
  await page.getByRole('button', { name: '修改API Key 前缀' }).click()
  await page.getByLabel('API Key 前缀', { exact: true }).fill('-bad')
  await page.getByRole('button', { name: '保存', exact: true }).click()
  await expect(page.getByRole('alert')).toContainText('以字母或数字开头')
  await page.getByLabel('API Key 前缀', { exact: true }).fill('team_1')
  await page.getByRole('button', { name: '保存', exact: true }).click()
  await expect(page.getByText('team_1', { exact: true })).toBeVisible()
  expect(state.writes.at(-1)).toEqual({ method: 'PATCH', path: '/api/account', body: { api_key_prefix: 'team_1' } })
  expect(state.writes).toHaveLength(2)

})

test('each kind of alert email can be turned off', async ({ page }) => {
  const state = await fixtures(page)
  await page.goto('/settings')
  await expect(page.getByText('监控发现问题时发送到 owner@example.invalid')).toBeVisible()
  const metering = page.getByRole('checkbox', { name: /计量异常/ })
  await expect(metering).toBeChecked()
  await metering.click()
  await expect(metering).not.toBeChecked()
  expect(state.writes.at(-1)).toEqual({ method: 'PATCH', path: '/api/account', body: { alert_emails: { authenticity: true, service: true, metering: false } } })
  await expect(page.getByRole('checkbox', { name: /真伪检测/ })).toBeChecked()
})

test('email changes ask for the new address and password first, and codes stay bound to the address', async ({ page }) => {
  const state = await fixtures(page)
  await page.goto('/settings')
  await page.getByRole('button', { name: '修改登录邮箱' }).click()
  const address = page.getByPlaceholder('新的登录邮箱')
  const send = page.getByRole('button', { name: '发送验证码' })
  await address.fill('owner@example.invalid')
  await page.getByLabel('当前密码', { exact: true }).fill('current-password-value')
  await send.click()
  await expect(page.getByRole('alert')).toHaveText('请输入与当前邮箱不同的新邮箱')
  await address.fill('new@example.invalid')
  await page.getByLabel('当前密码', { exact: true }).press('Enter')
  await expect(page.getByText('验证码已发送至 new@example.invalid，10 分钟内有效。')).toBeVisible()
  expect(state.writes).toEqual([{ method: 'POST', path: '/api/me/email-verification', body: { email: 'new@example.invalid', current_password: 'current-password-value' } }])
  await expect(page.getByRole('button', { name: /后可重发/ })).toBeDisabled()
  // Changing the address invalidates the code; restoring it needs a new code.
  await address.fill('other@example.invalid')
  await expect(page.getByLabel('邮箱验证码', { exact: true })).toHaveCount(0)
  await address.fill('new@example.invalid')
  await expect(page.getByLabel('邮箱验证码', { exact: true })).toHaveCount(0)
})

test('confirming the email code updates the address and explains the sign-out', async ({ page }) => {
  const state = await fixtures(page)
  await page.goto('/settings')
  await page.getByRole('button', { name: '修改登录邮箱' }).click()
  await page.getByPlaceholder('新的登录邮箱').fill('New@Example.invalid')
  await page.getByLabel('当前密码', { exact: true }).fill('current-password-value')
  await page.getByRole('button', { name: '发送验证码' }).click()
  const code = page.getByLabel('邮箱验证码', { exact: true })
  await code.fill('12a')
  await code.press('Enter')
  await expect(page.getByRole('alert')).toHaveText('请输入 6 位验证码')
  await code.fill('123456')
  await code.press('Enter')
  await expect(page.getByText('登录邮箱已更新为 new@example.invalid，其他设备上的登录已退出。')).toBeVisible()
  await expect(page.getByText('new@example.invalid', { exact: true })).toBeVisible()
  expect(state.writes.at(-1)).toEqual({ method: 'POST', path: '/api/me/email-verification/confirm', body: { request_id: 'request-1', email: 'new@example.invalid', code: '123456' } })
})

test('sign-in devices can be recognized and signed out', async ({ page }, testInfo) => {
  const state = await fixtures(page)
  await page.goto('/settings')
  const devices = page.getByRole('list', { name: '登录设备' })
  await expect(devices.getByRole('listitem')).toHaveCount(3)
  await expect(devices.getByRole('listitem').first()).toContainText('Chrome · macOS')
  await expect(devices.getByRole('listitem').first()).toContainText('当前设备')
  await expect(devices).toContainText('Safari · iOS')
  await expect(devices).toContainText('未知设备')
  await page.screenshot({ path: testInfo.outputPath('account-settings-desktop.png'), fullPage: true })
  await page.getByRole('button', { name: '退出 Safari · iOS 的登录' }).click()
  await expect(devices.getByRole('listitem')).toHaveCount(2)
  expect(state.writes).toEqual([{ method: 'DELETE', path: '/api/me/sessions/phone', body: null }])
  await page.getByRole('button', { name: '退出其他设备' }).click()
  await expect(page.getByText('已退出 1 个其他设备。')).toBeVisible()
  await expect(devices.getByRole('listitem')).toHaveCount(1)
  await expect(page.getByRole('button', { name: '退出其他设备' })).toBeDisabled()
  await page.setViewportSize({ width: 390, height: 844 })
  await page.screenshot({ path: testInfo.outputPath('account-settings-mobile.png'), fullPage: true })
})

async function authFixtures(page: Page) {
  const state = { writes: [] as { path: string; body: unknown }[] }
  await page.route('**/api/**', async route => {
    const path = new URL(route.request().url()).pathname
    const send = (body: unknown, status = 200) => route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) })
    if (path === '/api/me') return send({ error: { message: 'unauthorized' } }, 401)
    if (path === '/api/auth/registration') return send({ enabled: true, domain_restricted: false })
    if (path === '/api/setup') return send({ needed: false })
    state.writes.push({ path, body: route.request().postDataJSON() })
    if (path === '/api/auth/login') return send({ error: { code: 'unauthorized', message: 'unauthorized' } }, 401)
    if (path === '/api/auth/password-reset') return send({ request_id: 'reset-1', expires_in: 600, retry_after: 60 })
    if (path === '/api/auth/password-reset/confirm') return send({ ok: true })
    return send({}, 404)
  })
  await page.goto('/login')
  return state
}

test('failed sign-ins use a neutral message and passwords can be reset by email', async ({ page }) => {
  const state = await authFixtures(page)
  await page.getByLabel('邮箱', { exact: true }).fill('owner@example.invalid')
  await page.getByLabel('密码', { exact: true }).fill('wrong-password-value')
  await page.getByRole('button', { name: '登录', exact: true }).click()
  await expect(page.getByText('邮箱或密码不正确')).toBeVisible()
  await page.getByRole('button', { name: '忘记密码？' }).click()
  await expect(page.getByRole('heading', { name: '重置密码' })).toBeVisible()
  await expect(page.getByLabel('登录邮箱', { exact: true })).toHaveValue('owner@example.invalid')
  await page.getByRole('button', { name: '发送验证码' }).click()
  await expect(page.getByText('如果 owner@example.invalid 已注册，验证码会在几分钟内送达，10 分钟内有效。')).toBeVisible()
  await page.getByLabel('邮箱验证码', { exact: true }).fill('654321')
  await page.getByLabel('新密码', { exact: true }).fill('reset-password-value')
  await page.getByLabel('确认新密码', { exact: true }).fill('different-password-value')
  await page.getByRole('button', { name: '重置密码', exact: true }).click()
  await expect(page.getByRole('alert')).toHaveText('两次输入的新密码不一致')
  await page.getByLabel('确认新密码', { exact: true }).fill('reset-password-value')
  await page.getByRole('button', { name: '重置密码', exact: true }).click()
  await expect(page.getByText('密码已重置，请使用新密码登录。')).toBeVisible()
  await expect(page.getByLabel('邮箱', { exact: true })).toHaveValue('owner@example.invalid')
  expect(state.writes.slice(1)).toEqual([
    { path: '/api/auth/password-reset', body: { email: 'owner@example.invalid' } },
    { path: '/api/auth/password-reset/confirm', body: { request_id: 'reset-1', email: 'owner@example.invalid', code: '654321', new_password: 'reset-password-value' } },
  ])
})
