import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { ipc, type Session } from './lib/ipc'
import { AgentsIcon, BrandTile, KeyIcon, SettingsIcon } from './components/icons'
import { WelcomePage } from './pages/welcome'
import { AgentsPage } from './pages/agents'
import { KeysPage } from './pages/keys'
import { SettingsPage } from './pages/settings'

type Page = 'agents' | 'keys' | 'settings'

export function App() {
  const session = useQuery({ queryKey: ['session'], queryFn: ipc.session })

  return <div className="app">
    <TitleBar />
    {session.isPending ? <div className="login" />
      : session.isError ? <div className="login"><div className="banner banner-error">{session.error.message}</div><button className="btn" onClick={() => session.refetch()}>重试</button></div>
      : session.data.key_count > 0 ? <Shell session={session.data} />
      : <WelcomePage />}
  </div>
}

/** The native title bar is hidden; this strip keeps the window draggable. */
function TitleBar() {
  return <header className="titlebar" data-tauri-drag-region>
    <span className="titlebar-title" data-tauri-drag-region>OPENVAIR DESKTOP</span>
  </header>
}

function Shell({ session }: { session: Session }) {
  const [page, setPage] = useState<Page>('agents')
  const nav: { id: Page; label: string; icon: React.ReactNode }[] = [
    { id: 'agents', label: 'Agent', icon: <AgentsIcon /> },
    { id: 'keys', label: 'Key', icon: <KeyIcon /> },
  ]
  return <div className="body">
    <nav className="rail" aria-label="主导航">
      <div style={{ marginBottom: 14 }}><BrandTile /></div>
      {nav.map(item => <button key={item.id} className="rail-btn" aria-current={page === item.id ? 'page' : undefined} onClick={() => setPage(item.id)}>{item.icon}{item.label}</button>)}
      <button className="rail-btn" aria-current={page === 'settings' ? 'page' : undefined} onClick={() => setPage('settings')} style={{ marginTop: 'auto' }}><SettingsIcon />设置</button>
    </nav>
    {page === 'agents' && <AgentsPage session={session} />}
    {page === 'keys' && <KeysPage session={session} />}
    {page === 'settings' && <SettingsPage session={session} />}
  </div>
}
