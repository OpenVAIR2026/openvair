import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import '@fontsource-variable/geist'
import '@fontsource-variable/geist-mono'
import './styles.css'
import { App } from './App'

// Local commands are cheap; refetch on focus keeps cards current when the
// user edits a config file or a key in the console.
const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false, refetchOnWindowFocus: true } } })

// In a plain browser during development, fake the Rust side.
if (import.meta.env.DEV && !('__TAURI_INTERNALS__' in window)) (await import('./lib/mock')).installMock()

createRoot(document.getElementById('root')!).render(<StrictMode><QueryClientProvider client={queryClient}><App /></QueryClientProvider></StrictMode>)
