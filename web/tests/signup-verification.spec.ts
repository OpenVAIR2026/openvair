import { expect, test, type Page } from '@playwright/test'

// Sign-up exists only on the hosted service; run with VITE_EDITION=cloud.
test.skip(process.env.VITE_EDITION !== 'cloud', 'cloud edition only')

async function fixtures(page: Page, emailVerification: boolean) {
  const state = { signedIn: false, writes: [] as { path: string; body: any }[] }
  await page.route('**/api/**', async route => {
    const path = new URL(route.request().url()).pathname
    const send = (body: unknown, status = 200) => route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) })
    if (route.request().method() !== 'GET') {
      const body = route.request().postDataJSON()
      state.writes.push({ path, body })
      if (path === '/api/auth/register/code') return send({ request_id: 'request-1', expires_in: 600, retry_after: 60 })
      if (path === '/api/auth/register') { state.signedIn = true; return send({ user_id: 'user', account_id: 'account', email: body.email }, 201) }
      return send({})
    }
    if (path === '/api/auth/registration') return send({ enabled: true, domain_restricted: false, email_verification: emailVerification })
    if (path === '/api/me') return state.signedIn ? send({ user_id: 'user', email: 'new@example.invalid', account_id: 'account', is_super_admin: false }) : send({ error: { message: '登录已过期' } }, 401)
    return send([])
  })
  return state
}

const openSignup = async (page: Page) => {
  await page.goto('/login')
  await page.getByRole('button', { name: '还没有帐号？创建帐号' }).click()
}

test('sign-up sends a code to the email and submits it with the account', async ({ page }) => {
  const state = await fixtures(page, true)
  await openSignup(page)
  await page.getByRole('button', { name: '发送验证码' }).click()
  await expect(page.getByText('请输入有效邮箱')).toBeVisible()
  expect(state.writes).toEqual([])

  await page.getByLabel('邮箱', { exact: true }).fill('New@Example.invalid')
  await page.getByRole('button', { name: '发送验证码' }).click()
  await expect(page.getByRole('status')).toContainText('验证码已发送到 new@example.invalid')
  await expect(page.getByRole('button', { name: /重新发送 \(\d+s\)/ })).toBeDisabled()
  expect(state.writes).toEqual([{ path: '/api/auth/register/code', body: { email: 'new@example.invalid' } }])

  await page.getByLabel('帐号名称').fill('Acme AI')
  await page.getByLabel('密码', { exact: true }).fill('signup-password')
  await page.getByRole('button', { name: '创建帐号' }).click()
  await expect(page.getByText('请输入 6 位验证码')).toBeVisible()

  await page.getByLabel('邮箱验证码').fill('482 917')
  await expect(page.getByLabel('邮箱验证码')).toHaveValue('482917')
  await expect(page.getByText('请输入 6 位验证码')).toHaveCount(0)
  await page.screenshot({ path: 'test-results/signup-verification.png' })
  await page.getByRole('button', { name: '创建帐号' }).click()
  await expect(page).toHaveURL(/\/overview$/)
  expect(state.writes.at(-1)).toEqual({ path: '/api/auth/register', body: { email: 'New@Example.invalid', password: 'signup-password', account_name: 'Acme AI', request_id: 'request-1', code: '482917' } })
})

test('editing the email drops the code sent to the previous address', async ({ page }) => {
  const state = await fixtures(page, true)
  await openSignup(page)
  await page.getByLabel('邮箱', { exact: true }).fill('first@example.invalid')
  await page.getByRole('button', { name: '发送验证码' }).click()
  await expect(page.getByRole('status')).toContainText('first@example.invalid')
  await page.getByLabel('邮箱', { exact: true }).fill('second@example.invalid')
  await expect(page.getByRole('status')).toHaveCount(0)
  await expect(page.getByRole('button', { name: '发送验证码' })).toBeEnabled()
  await page.getByLabel('邮箱验证码').fill('123456')
  await page.getByLabel('帐号名称').fill('Acme AI')
  await page.getByLabel('密码', { exact: true }).fill('signup-password')
  await page.getByRole('button', { name: '创建帐号' }).click()
  await expect(page.getByText('请先获取邮箱验证码')).toBeVisible()
  expect(state.writes.map(write => write.path)).toEqual(['/api/auth/register/code'])
})

test('sign-up skips the code when verification is off', async ({ page }) => {
  const state = await fixtures(page, false)
  await openSignup(page)
  await expect(page.getByLabel('邮箱验证码')).toHaveCount(0)
  await expect(page.getByRole('button', { name: '发送验证码' })).toHaveCount(0)
  await page.getByLabel('邮箱', { exact: true }).fill('new@example.invalid')
  await page.getByLabel('帐号名称').fill('Acme AI')
  await page.getByLabel('密码', { exact: true }).fill('signup-password')
  await page.getByRole('button', { name: '创建帐号' }).click()
  await expect(page).toHaveURL(/\/overview$/)
  expect(state.writes).toEqual([{ path: '/api/auth/register', body: { email: 'new@example.invalid', password: 'signup-password', account_name: 'Acme AI' } }])
})
