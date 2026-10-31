import { createContext, useContext, useMemo } from 'react'
import { QueryClient, useQuery } from '@tanstack/react-query'
import { api, type AccountLimits, type RoutingSettings, type Protocol, type AuditEvent, type Channel, type ModelCatalogEntry, type ModelPrice, type Provider, type UnpricedModels, type MonitoringSummary, type QualityMonitor, type Account, type User, type VirtualKey, type KeyGroup } from './api'

export const queryClient = new QueryClient({ defaultOptions: { queries: { staleTime: 20_000, retry: false }, mutations: { retry: false } } })
export const UserContext = createContext<User | null>(null)
export function useUser() {
  const user = useContext(UserContext)
  if (!user) throw new Error('未选择帐号')
  return user
}
/** The address colleagues reach this server at: the administrator's 对外访问地址, else the one this browser opened. */
export function useOrigin() {
  const server = useQuery({ queryKey: ['server'], queryFn: ({ signal }) => api<{ public_url: string | null }>('/server', { signal }), staleTime: 300_000 })
  return server.data?.public_url ?? window.location.origin
}
export function useResource<T>(name: string, path = `/${name}`) {
  const user = useUser()
  return useQuery({ queryKey: [name, user.account_id], queryFn: ({ signal }) => api<T>(path, { signal }) })
}
export const useAccount = () => useResource<Account>('account')
export const useChannels = () => useResource<Channel[]>('channels')
export const useProviders = () => useResource<Provider[]>('providers')
export const useKeys = () => useResource<VirtualKey[]>('keys')
export const useKeyGroups = () => useResource<KeyGroup[]>('key-groups', '/keys/groups')
export const useModelCatalog = () => useResource<ModelCatalogEntry[]>('model-catalog')
/** Concrete model names for pickers; prefix rules are typed, not suggested. */
export function useModelSuggestions(models: string[], protocol?: Protocol) {
  const catalog = useModelCatalog()
  return useMemo(() => {
    const options = new Map<string, string>()
    catalog.data?.filter(item => item.enabled && (!protocol || item.protocols?.includes(protocol)) && !item.pattern.endsWith('*'))
      .forEach(item => options.set(item.pattern, item.label))
    models.map(model => model.trim()).filter(model => model && !model.endsWith('*'))
      .forEach(model => { if (!options.has(model)) options.set(model, model) })
    return [...options].map(([value, label]) => ({ value, label }))
  }, [catalog.data, models, protocol])
}
export const useMonitoringSummary = () => useResource<MonitoringSummary>('monitoring-summary', '/monitoring/summary')
export const useAccountLimits = () => useResource<AccountLimits>('account-limits', '/platform/account-limits')
export const useRoutingSettings = () => useResource<RoutingSettings>('routing-settings', '/platform/routing-settings')
export function useUnpricedModels() {
  const user = useUser()
  return useQuery({
    queryKey: ['unpriced-models', user.account_id],
    queryFn: ({ signal }) => api<UnpricedModels>('/platform/model-pricing/unpriced', { signal }),
    enabled: user.is_super_admin,
  })
}
export const useAuditLog = () => useResource<AuditEvent[]>('audit', '/audit')
export function useModelPricingCatalog() {
  const user = useUser()
  return useQuery({
    queryKey: ['model-pricing-catalog', user.account_id],
    queryFn: ({ signal }) => api<ModelPrice[]>('/platform/model-pricing/catalog', { signal }),
    enabled: user.is_super_admin,
    staleTime: 10 * 60 * 1000,
  })
}

export async function refreshModels() {
  await Promise.all(['model-catalog', 'channels', 'keys', 'unpriced-models', 'quality-monitors', 'monitoring-summary'].map(name => queryClient.invalidateQueries({ queryKey: [name] })))
}

function useMonitors(kind: QualityMonitor['kind']) {
  const user = useUser()
  return useQuery({ queryKey: ['quality-monitors', user.account_id], queryFn: ({ signal }) => api<QualityMonitor[]>('/quality-monitors', { signal }), select: data => data.filter(item => item.kind === kind), refetchInterval: query => query.state.data?.some(item => item.job_status === 'queued' || item.job_status === 'running') ? 2000 : 15000 })
}
/** Service-check monitors: the ones that run service checks and steer routing. */
export const useQualityMonitors = () => useMonitors('service')
export const useAuthenticityMonitors = () => useMonitors('authenticity')
