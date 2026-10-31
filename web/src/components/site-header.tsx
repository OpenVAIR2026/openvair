import type { ReactNode } from 'react'
import { useQuery } from '@tanstack/react-query'
import { ArrowUpRight } from 'lucide-react'
import { Link, NavLink } from 'react-router-dom'
import { api, type User } from '../lib/api'
import { BrandMark, BrandName } from './brand'
import { CLOUD, PRODUCT } from '../lib/edition'
import { Button } from './ui/button'
import './site-header.css'

const tabs = CLOUD ? [
  { to: '/', label: '首页' },
  { to: '/verify', label: '模型验证' },
  { to: '/docs', label: '使用文档' },
  { to: '/download', label: '桌面客户端' },
] : [{ to: '/docs', label: '使用文档' }]

export function SiteHeader({ actions }: { actions?: ReactNode }) {
  const me = useQuery({ queryKey: ['me'], queryFn: ({ signal }) => api<User>('/me', { signal }), retry: false })
  const signedIn = me.isSuccess
  return <header className="site-header">
    <Link className="brand site-brand" to="/" aria-label={`${PRODUCT} 首页`}><BrandMark /><BrandName /></Link>
    <nav className="site-nav" aria-label="站点导航">
      {tabs.map(tab => <NavLink key={tab.to} to={tab.to} end={tab.to === '/'} onClick={tab.to === '/' ? () => window.scrollTo({ top: 0, behavior: 'smooth' }) : undefined}>{tab.label}</NavLink>)}
    </nav>
    <div className="site-actions">
      {actions ?? <NavLink className="site-link" to="/my-key">查看我的 Key</NavLink>}
      {!signedIn && <Link className="site-login" to="/login">登录 <ArrowUpRight aria-hidden="true" size={15} /></Link>}
      <Button asChild size="sm" className="site-cta"><Link to={signedIn ? '/overview' : '/login'}>{signedIn ? '进入控制台' : '开始配置'}</Link></Button>
    </div>
  </header>
}
