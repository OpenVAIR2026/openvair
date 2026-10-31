import { StrictMode, Suspense, lazy } from 'react'
import { createRoot } from 'react-dom/client'
import { QueryClientProvider } from '@tanstack/react-query'
import { BrowserRouter, Navigate, Route, Routes } from 'react-router-dom'
import { queryClient } from './lib/queries'
import { CLOUD } from './lib/edition'
import { HomePage } from './pages/home'
import './styles.css'

// A deploy replaces the hashed chunks, so a tab opened before it fails to load
// the next lazy page. Reload once to pick up the new index.html.
window.addEventListener('vite:preloadError', event => {
  const key = 'vair:chunk-reload'
  const last = Number(sessionStorage.getItem(key) ?? 0)
  if (Date.now() - last < 10_000) return
  sessionStorage.setItem(key, String(Date.now()))
  event.preventDefault()
  location.reload()
})

const App = lazy(() => import('./App').then(m => ({ default: m.App })))
const AuthPage = lazy(() => import('./pages/auth').then(m => ({ default: m.AuthPage })))
const OverviewPage = lazy(() => import('./pages/overview').then(m => ({ default: m.OverviewPage })))
const ChannelsPage = lazy(() => import('./pages/channels').then(m => ({ default: m.ChannelsPage })))
const KeysPage = lazy(() => import('./pages/keys').then(m => ({ default: m.KeysPage })))
const UsagePage = lazy(() => import('./pages/usage').then(m => ({ default: m.UsagePage })))
const SettingsPage = lazy(() => import('./pages/settings').then(m => ({ default: m.SettingsPage })))
const MonitoringPage = lazy(() => import('./pages/monitoring').then(m => ({ default: m.MonitoringPage })))
const PlatformPage = lazy(() => import('./pages/platform').then(m => ({ default: m.PlatformPage })))
const PlatformEmailPage = lazy(() => import('./pages/platform').then(m => ({ default: m.PlatformEmailPage })))
const PlatformModelsPage = lazy(() => import('./pages/platform').then(m => ({ default: m.PlatformModelsPage })))
const PlatformRoutingPage = lazy(() => import('./pages/platform').then(m => ({ default: m.PlatformRoutingPage })))
const PlatformPricingPage = lazy(() => import('./pages/platform').then(m => ({ default: m.PlatformPricingPage })))
const MyKeyPage = lazy(() => import('./pages/my-key').then(m => ({ default: m.MyKeyPage })))
const ClientActivatePage = lazy(() => import('./pages/client-activate').then(m => ({ default: m.ClientActivatePage })))
const PlatformBaselinesPage = lazy(() => import('./pages/model-baselines').then(m => ({ default: m.PlatformBaselinesPage })))
const PlatformPublicUrlPage = /*#__PURE__*/ lazy(() => import('./pages/platform-admin').then(m => ({ default: m.PlatformPublicUrlPage })))
const PlatformHealthPage = lazy(() => import('./pages/platform-admin').then(m => ({ default: m.PlatformHealthPage })))
const PlatformUnknownCostPage = lazy(() => import('./pages/platform').then(m => ({ default: m.PlatformUnknownCostPage })))
const DocsPage = lazy(() => import('./pages/docs').then(m => ({ default: m.DocsPage })))
const ActivityPage = lazy(() => import('./pages/activity').then(m => ({ default: m.ActivityPage })))
const SetupPage = lazy(() => import('./pages/setup').then(m => ({ default: m.SetupPage })))

// Pages of the hosted service only; their APIs are not mounted in OpenVAIR.
const PlatformLimitsPage = /*#__PURE__*/ lazy(() => import('./pages/platform').then(m => ({ default: m.PlatformLimitsPage })))
const DownloadPage = /*#__PURE__*/ lazy(() => import('./pages/download').then(m => ({ default: m.DownloadPage })))
const PlatformAccountsPage = /*#__PURE__*/ lazy(() => import('./pages/platform-admin').then(m => ({ default: m.PlatformAccountsPage })))
const PlatformRegistrationPage = /*#__PURE__*/ lazy(() => import('./pages/platform-admin').then(m => ({ default: m.PlatformRegistrationPage })))
const PlatformPublicVerifyPage = /*#__PURE__*/ lazy(() => import('./pages/platform-admin').then(m => ({ default: m.PlatformPublicVerifyPage })))
const VerifyPage = /*#__PURE__*/ lazy(() => import('./pages/verify').then(m => ({ default: m.VerifyPage })))
const VerifyReportPage = /*#__PURE__*/ lazy(() => import('./pages/verify').then(m => ({ default: m.VerifyReportPage })))
const SharePage = /*#__PURE__*/ lazy(() => import('./pages/share').then(m => ({ default: m.SharePage })))

createRoot(document.getElementById('root')!).render(<StrictMode><QueryClientProvider client={queryClient}><BrowserRouter><Suspense fallback={null}><Routes>
  {CLOUD ? <>
    <Route path="/" element={<HomePage />} />
    <Route path="/download" element={<DownloadPage />} />
    <Route path="/verify" element={<VerifyPage />} />
    <Route path="/verify/r/:token" element={<VerifyReportPage />} />
    <Route path="/client/activate" element={<ClientActivatePage />} />
  </> : <>
    <Route path="/" element={<Navigate to="/overview" replace />} />
    <Route path="/setup" element={<SetupPage />} />
  </>}
  <Route path="/login" element={<AuthPage />} />
  <Route path="/my-key/*" element={<MyKeyPage />} />
  <Route path="/docs/*" element={<DocsPage />} />
  <Route element={<App />}>
    <Route path="/overview" element={<OverviewPage />} />
    <Route path="/channels" element={<ChannelsPage />} />
    <Route path="/monitoring" element={<MonitoringPage />} />
    <Route path="/routes" element={<Navigate to="/channels" replace />} />
    <Route path="/keys" element={<KeysPage />} />
    <Route path="/usage" element={<UsagePage />} />
    <Route path="/settings" element={<SettingsPage />} />
    <Route path="/platform" element={<PlatformPage />} />
    <Route path="/platform/email-settings" element={<PlatformEmailPage />} />
    <Route path="/platform/model-catalog" element={<PlatformModelsPage />} />
    <Route path="/platform/routing-settings" element={<PlatformRoutingPage />} />
    <Route path="/platform/public-url" element={<PlatformPublicUrlPage />} />
    <Route path="/platform/health" element={<PlatformHealthPage />} />
    <Route path="/platform/unknown-cost" element={<PlatformUnknownCostPage />} />
    <Route path="/platform/model-pricing" element={<PlatformPricingPage />} />
    <Route path="/activity" element={<ActivityPage />} />
    {CLOUD && <>
      <Route path="/platform/accounts" element={<PlatformAccountsPage />} />
      <Route path="/platform/registration" element={<PlatformRegistrationPage />} />
      <Route path="/platform/account-limits" element={<PlatformLimitsPage />} />
      <Route path="/platform/model-baselines" element={<PlatformBaselinesPage />} />
      <Route path="/platform/public-verify" element={<PlatformPublicVerifyPage />} />
      <Route path="/share" element={<SharePage />} />
    </>}
  </Route>
  <Route path="*" element={<Navigate to="/" replace />} />
</Routes></Suspense></BrowserRouter></QueryClientProvider></StrictMode>)
