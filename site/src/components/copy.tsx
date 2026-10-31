import { useState } from 'react'
import { Check, Copy } from 'lucide-react'
import { useLang } from '../i18n'

/** The Clipboard API only exists on HTTPS and localhost; a hidden textarea covers plain-HTTP previews. */
export async function copyText(text: string) {
  if (navigator.clipboard && window.isSecureContext) return navigator.clipboard.writeText(text)
  const area = document.createElement('textarea')
  area.value = text
  area.style.position = 'fixed'
  area.style.opacity = '0'
  document.body.append(area)
  area.select()
  document.execCommand('copy')
  area.remove()
}

/** A one-line shell command with a copy button, used for the install command. */
export function Command({ text, label }: { text: string; label?: string }) {
  const { t: { copy } } = useLang()
  const [copied, setCopied] = useState(false)
  return <div className="command">
    <span className="command-label">{label ?? copy.terminal}</span>
    <code><span className="command-prompt" aria-hidden="true">$</span>{text}</code>
    <button type="button" onClick={() => copyText(text).then(() => { setCopied(true); setTimeout(() => setCopied(false), 1600) })} aria-label={copied ? copy.copied : copy.copyCommand}>
      {copied ? <Check aria-hidden="true" /> : <Copy aria-hidden="true" />}<span>{copied ? copy.copied : copy.copy}</span>
    </button>
  </div>
}
