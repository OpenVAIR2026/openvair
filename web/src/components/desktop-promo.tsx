import { useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { Download, X } from 'lucide-react'
import { BrandMark } from './brand'
import { CLOUD } from '../lib/edition'
import { Button } from './ui/button'

const DISMISSED = 'vair.desktop-promo.dismissed'

function readDismissed() {
  try { return localStorage.getItem(DISMISSED) === '1' } catch { return false }
}

/** Points key owners to VAIR Desktop. Hidden per browser once dismissed. */
export function DesktopPromo() {
  const navigate = useNavigate()
  const [dismissed, setDismissed] = useState(readDismissed)
  // Installers are only published by the hosted service.
  if (dismissed || !CLOUD) return null
  const dismiss = () => {
    try { localStorage.setItem(DISMISSED, '1') } catch { /* private mode: hide for this visit only */ }
    setDismissed(true)
  }
  // One slim row below the key list, so the keys come first.
  return <section aria-labelledby="desktop-promo-title" className="desktop-promo desktop-promo-slim mt-4 flex items-center gap-3 rounded-lg border border-primary/30 py-2 pr-2 pl-3">
    <BrandMark />
    <p id="desktop-promo-title" className="min-w-0 flex-1 truncate text-sm"><span className="font-medium">VAIR Desktop</span><span className="text-muted-foreground"> · 自动配置本机 Agent</span></p>
    <Button type="button" variant="outline" size="sm" className="shrink-0" onClick={() => navigate('/download')}><Download aria-hidden="true" />下载客户端</Button>
    <Button type="button" variant="ghost" size="icon-xs" className="shrink-0 text-muted-foreground" aria-label="不再显示客户端下载提示" onClick={dismiss}><X /></Button>
  </section>
}

/** The same offer at the end of the connect guide. Opens a new tab so the guide stays open. */
export function DesktopShortcut() {
  if (!CLOUD) return null
  return <section aria-labelledby="desktop-shortcut-title" className="desktop-promo flex flex-wrap items-center gap-3 rounded-lg border border-primary/30 p-3 sm:flex-nowrap">
    <BrandMark />
    <div className="min-w-0 flex-1">
      <h3 id="desktop-shortcut-title" className="text-sm font-medium">用 VAIR Desktop 自动配置</h3>
      <p className="mt-0.5 text-xs leading-5 text-muted-foreground">免去手动配置</p>
    </div>
    <Button type="button" variant="outline" size="sm" className="shrink-0" onClick={() => window.open('/download', '_blank', 'noopener')}><Download aria-hidden="true" />下载客户端</Button>
  </section>
}
