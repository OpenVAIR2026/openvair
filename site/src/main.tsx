import { lazy, StrictMode, Suspense } from 'react'
import { createRoot } from 'react-dom/client'
import { BrowserRouter, Route, Routes } from 'react-router-dom'
import { Layout } from './components/layout'
import { HomePage } from './pages/home'

// Pages that render markdown load it separately, so the landing page stays small.
const DeployPage = lazy(() => import('./pages/deploy').then(module => ({ default: module.DeployPage })))
const DocsPage = lazy(() => import('./pages/docs').then(module => ({ default: module.DocsPage })))
const DownloadPage = lazy(() => import('./pages/download').then(module => ({ default: module.DownloadPage })))
import './styles.css'

/** The same pages at the root (Chinese) and under /en (English); the URL decides the language. */
const pages = (prefix: string) => <>
  <Route path={prefix || '/'} element={<HomePage />} />
  <Route path={`${prefix}/deploy`} element={<DeployPage />} />
  <Route path={`${prefix}/download`} element={<DownloadPage />} />
  <Route path={`${prefix}/docs/*`} element={<DocsPage />} />
</>

createRoot(document.getElementById('root')!).render(<StrictMode>
  <BrowserRouter>
    <Layout>
      <Suspense fallback={<div className="page-loading" />}><Routes>
        {pages('')}
        {pages('/en')}
        <Route path="*" element={<DocsPage missing />} />
      </Routes></Suspense>
    </Layout>
  </BrowserRouter>
</StrictMode>)
