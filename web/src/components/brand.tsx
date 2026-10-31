import { useId } from 'react'
import { cn } from '../lib/utils'
import { PRODUCT } from '../lib/edition'

// Three channels converging on one node: the outer strokes also form the V of VAIR.
function BrandSvg() {
  const gradient = useId()
  return <svg className="brand-svg" viewBox="2.5 2 19 19" aria-hidden="true">
    <defs><linearGradient id={gradient} gradientUnits="userSpaceOnUse" x1="4" y1="4" x2="20" y2="20"><stop offset="0" stopColor="#d6fff8" /><stop offset=".55" stopColor="#78d9d0" /><stop offset="1" stopColor="#3aa8d8" /></linearGradient></defs>
    <path d="M12 6.5V18.5" stroke={`url(#${gradient})`} strokeOpacity=".45" strokeWidth="1.6" strokeLinecap="round" fill="none" />
    <path d="M4.5 5L12 18.5L19.5 5" stroke={`url(#${gradient})`} strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round" fill="none" />
    <circle cx="4.5" cy="5" r="1.7" fill="#d6fff8" />
    <circle cx="12" cy="5" r="1.4" fill="#78d9d0" fillOpacity=".7" />
    <circle cx="19.5" cy="5" r="1.7" fill="#5cc4dc" />
    <circle className="brand-node" cx="12" cy="18.5" r="2.3" fill="#ecfffb" />
  </svg>
}

export function BrandMark({ className }: { className?: string }) {
  return <span className={cn('brand-mark', className)}><BrandSvg /></span>
}

export function BrandName({ className }: { className?: string }) {
  return <span className={cn('brand-name', className)}>{PRODUCT}</span>
}
