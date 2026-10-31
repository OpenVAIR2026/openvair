import { useId, type ReactNode } from 'react'

function Icon({ size = 18, children }: { size?: number; children: ReactNode }) {
  return <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">{children}</svg>
}

export const AgentsIcon = () => <Icon size={20}><circle cx="12" cy="12" r="3" /><circle cx="5" cy="5" r="2" /><circle cx="19" cy="5" r="2" /><circle cx="19" cy="19" r="2" /><path d="M6.5 6.5l3.4 3.4M17.5 6.5l-3.4 3.4M17.5 17.5l-3.4-3.4" /></Icon>
export const KeyIcon = () => <Icon size={20}><circle cx="8" cy="15" r="4" /><path d="M10.8 12.2L20 3M16 7l3 3M14 9l2 2" /></Icon>
export const SettingsIcon = () => <Icon size={20}><path d="M4 7h10M18 7h2M4 17h4M12 17h8" /><circle cx="16" cy="7" r="2" /><circle cx="10" cy="17" r="2" /></Icon>
export const CheckIcon = ({ size = 16 }: { size?: number }) => <Icon size={size}><path d="M5 12l5 5L20 7" /></Icon>
export const CloseIcon = () => <Icon><path d="M6 6l12 12M18 6L6 18" /></Icon>
export const WarnIcon = () => <Icon size={16}><path d="M12 9v4M12 17h.01M10.3 3.9L1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0z" /></Icon>
export const ExternalIcon = () => <Icon size={16}><path d="M14 4h6v6M20 4l-9 9M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5" /></Icon>
export const RefreshIcon = () => <Icon size={16}><path d="M20 11a8 8 0 1 0-2.3 5.7M20 4v7h-7" /></Icon>
export const SearchIcon = () => <Icon size={16}><circle cx="11" cy="11" r="7" /><path d="M20 20l-3.5-3.5" /></Icon>
export const MoreIcon = () => <Icon><circle cx="5" cy="12" r="1" /><circle cx="12" cy="12" r="1" /><circle cx="19" cy="12" r="1" /></Icon>
export const CopyIcon = () => <Icon size={16}><rect x="8" y="8" width="12" height="12" rx="2" /><path d="M16 8V5a1 1 0 0 0-1-1H5a1 1 0 0 0-1 1v10a1 1 0 0 0 1 1h3" /></Icon>
export const LockIcon = () => <Icon size={14}><rect x="5" y="11" width="14" height="10" rx="2" /><path d="M8 11V7a4 4 0 0 1 8 0v4" /></Icon>

/** The OpenVAIR V: three channels converging on one node. Same artwork as the website's brand mark. */
export function BrandMark({ size = 24 }: { size?: number }) {
  const id = useId()
  return <svg viewBox="2.5 2 19 19" width={size} height={size} style={{ overflow: 'visible' }} aria-hidden="true">
    <defs><linearGradient id={id} gradientUnits="userSpaceOnUse" x1="4" y1="4" x2="20" y2="20"><stop offset="0" stopColor="#d6fff8" /><stop offset=".55" stopColor="#78d9d0" /><stop offset="1" stopColor="#3aa8d8" /></linearGradient></defs>
    <path d="M12 6.5V18.5" stroke={`url(#${id})`} strokeOpacity=".45" strokeWidth="1.6" strokeLinecap="round" fill="none" />
    <path d="M4.5 5L12 18.5L19.5 5" stroke={`url(#${id})`} strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round" fill="none" />
    <circle cx="4.5" cy="5" r="1.7" fill="#d6fff8" />
    <circle cx="12" cy="5" r="1.4" fill="#78d9d0" fillOpacity=".7" />
    <circle cx="19.5" cy="5" r="1.7" fill="#5cc4dc" />
    <circle className="brand-node" cx="12" cy="18.5" r="2.3" fill="#ecfffb" />
  </svg>
}

/** The website's 36px brand tile. */
export function BrandTile() {
  return <span className="brand-mark" aria-label="OpenVAIR"><BrandMark /></span>
}
