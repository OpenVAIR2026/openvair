// Per-viewer onboarding conveniences. Storage can be unavailable, so every access falls back quietly.
const storageKey = (name: string, accountId: string) => `vair:onboarding:${name}:${accountId}`

function read(name: string, accountId: string) {
  try { return localStorage.getItem(storageKey(name, accountId)) === '1' } catch { return false }
}

function write(name: string, accountId: string) {
  try { localStorage.setItem(storageKey(name, accountId), '1') } catch { /* storage unavailable */ }
  window.dispatchEvent(new Event('vair:onboarding'))
}

export const connectionVerified = (accountId: string) => read('verified', accountId)
export const markConnectionVerified = (accountId: string) => write('verified', accountId)
export const onboardingDismissed = (accountId: string) => read('dismissed', accountId)
export const dismissOnboarding = (accountId: string) => write('dismissed', accountId)

export type ConnectionResult =
  | { status: 'ok'; models: number }
  | { status: 'unauthorized' }
  | { status: 'error'; message: string }

/** Lists the key's models through the gateway: this authenticates the key without calling any upstream. */
export async function testConnection(key: string, signal?: AbortSignal): Promise<ConnectionResult> {
  let response: Response
  try {
    response = await fetch('/v1/models', { headers: { authorization: `Bearer ${key}` }, cache: 'no-store', credentials: 'omit', signal })
  } catch (error) {
    if (signal?.aborted) throw error
    return { status: 'error', message: '无法连接网关，请检查网络后重试。' }
  }
  if (response.status === 401) return { status: 'unauthorized' }
  if (!response.ok) return { status: 'error', message: `网关返回 HTTP ${response.status}，请稍后重试。` }
  const body = await response.json().catch(() => null) as { data?: unknown[] } | null
  return { status: 'ok', models: Array.isArray(body?.data) ? body.data.length : 0 }
}
