import { useQuery } from '@tanstack/react-query'
import { Navigate, Outlet, useLocation, useNavigate, Link } from 'react-router-dom'
import { Activity, BookOpen, CircleGauge, Fingerprint, Globe, HeartPulse, History, KeyRound, LayoutDashboard, List, Mail, Network, Radar, Route, Settings, ShieldCheck, SlidersHorizontal, UserPlus, Users } from 'lucide-react'
import { LogoutOutlined } from '@ant-design/icons'
import { ProLayout, type MenuDataItem } from '@ant-design/pro-components'
import { Button as AntButton, ConfigProvider, Tooltip, theme as antdTheme } from 'antd'
import { Suspense, useState } from 'react'
import { api, ApiError, type User } from './lib/api'
import { queryClient, UserContext, useAccount } from './lib/queries'
import { ErrorBox, Loading } from './components/common'
import { BrandMark, BrandName } from './components/brand'
import { CLOUD, PRODUCT } from './lib/edition'

const baseNavigation: MenuDataItem[] = [
  { path: '/overview', name: '运行概览', icon: <LayoutDashboard aria-hidden="true" /> },
  { path: '/channels', name: '上游渠道', icon: <Network aria-hidden="true" /> },
  { path: '/keys', name: 'API Key', icon: <KeyRound aria-hidden="true" /> },
  { path: '/monitoring', name: '渠道监控', icon: <Radar aria-hidden="true" /> },
  { path: '/usage', name: '使用分析', icon: <CircleGauge aria-hidden="true" /> },
  { path: '/settings', name: '帐号设置', icon: <Settings aria-hidden="true" /> },
  { path: '/activity', name: '操作日志', icon: <History aria-hidden="true" /> },
]
// Sign-up, account limits, model baselines and public verification exist only on the hosted service.
const systemNavigation: MenuDataItem[] = [
  ...(CLOUD ? [{ path: '/platform/accounts', name: '帐号管理', icon: <Users aria-hidden="true" /> }, { path: '/platform/registration', name: '注册设置', icon: <UserPlus aria-hidden="true" /> }, { path: '/platform/account-limits', name: '帐号限制', icon: <SlidersHorizontal aria-hidden="true" /> }] : []),
  { path: '/platform/routing-settings', name: '路由设置', icon: <Route aria-hidden="true" /> },
  { path: '/platform/model-catalog', name: '模型管理', icon: <List aria-hidden="true" /> },
  ...(CLOUD ? [{ path: '/platform/model-baselines', name: '模型基准', icon: <Fingerprint aria-hidden="true" /> }, { path: '/platform/public-verify', name: '模型验证', icon: <ShieldCheck aria-hidden="true" /> }] : []),
  { path: '/platform/public-url', name: '访问地址', icon: <Globe aria-hidden="true" /> },
  { path: '/platform/email-settings', name: '邮箱设置', icon: <Mail aria-hidden="true" /> },
  { path: '/platform/health', name: '系统状态', icon: <HeartPulse aria-hidden="true" /> },
]
const consoleTheme = { algorithm: antdTheme.darkAlgorithm, token: { fontFamily: 'var(--font-console)', fontSize: 14, colorPrimary: '#78d9d0', colorInfo: '#78d9d0', borderRadius: 8, colorBgBase: '#0a1216', colorBgContainer: '#111b20', colorBgElevated: '#17232a', colorBorder: '#e2f6f41a', colorText: '#eef6f5', colorTextSecondary: '#9aaeb3' }, components: { Layout: { bodyBg: '#0a1216', siderBg: '#0a1216', headerBg: '#0a1216', triggerBg: '#111b20' }, Table: { headerBg: '#16222a', rowHoverBg: '#17232a' }, Card: { colorBgContainer: '#111b20' } } }
function Brand({ collapsed = false, link = false }: { collapsed?: boolean; link?: boolean }) {
  const className = `brand sidebar-brand${collapsed ? ' sidebar-brand-collapsed' : ''}`
  const content = <><BrandMark />{!collapsed && <div className="grid gap-1"><BrandName className="sidebar-brand-name" /><span className="sidebar-brand-tagline">BYOK GATEWAY</span></div>}</>
  // ProLayout wraps the logo prop in its own link on small screens, so only the sidebar header is a link.
  return link ? <Link className={className} to="/" aria-label={`${PRODUCT} 首页`}>{content}</Link> : <div className={className} aria-label={PRODUCT}>{content}</div>
}
function Shell({ user }: { user: User }) {
  const account = useAccount(); const navigate = useNavigate(); const [loggingOut, setLoggingOut] = useState(false); const [error, setError] = useState<Error | null>(null); const location = useLocation()
  const navigation = user.is_super_admin ? [...baseNavigation, { key: 'system', name: '系统管理', type: 'group', children: systemNavigation }] : baseNavigation
  async function logout() { setLoggingOut(true); try { await api('/auth/logout', { method: 'POST' }); await queryClient.cancelQueries(); queryClient.clear(); navigate('/login', { replace: true }) } catch (e) { setError(e as Error) } finally { setLoggingOut(false) } }
  return <div className="pro-console-shell min-h-dvh"><a className="skip-link" href="#main-content">跳到主要内容</a><ProLayout
    title={PRODUCT}
    logo={<Brand />}
    route={{ routes: navigation }}
    location={{ pathname: location.pathname }}
    navTheme="realDark"
    layout="side"
    siderMenuType="group"
    fixedHeader
    fixSiderbar
    siderWidth={240}
    contentWidth="Fluid"
    menuHeaderRender={(_, __, props) => props ? <div className="sidebar-brand-header"><Brand collapsed={props.collapsed} link />{CLOUD && !props.collapsed && <div className="sidebar-account-context"><span className="sidebar-account-label">当前帐号</span><span className="truncate">{account.data?.name ?? '加载中'}</span></div>}</div> : <Brand />}
    headerTitleRender={() => <span className="text-xs text-muted-foreground">{CLOUD ? `帐号控制台 · ${account.data?.name ?? '加载中'}` : `${PRODUCT} 控制台`}</span>}
    menuItemRender={(item, dom) => item.path ? <Link to={item.path} aria-label={item.name}>{dom}</Link> : dom}
    actionsRender={({ collapsed }) => [
      ...(!collapsed ? [<span key="transport" className="sidebar-transport hidden items-center gap-2 whitespace-nowrap text-xs text-muted-foreground md:flex"><Activity aria-hidden="true" className="size-3.5 text-primary" />协议原生透传</span>] : []),
      <Tooltip key="docs" title={collapsed ? '使用文档' : undefined} placement="right"><AntButton type="text" href="/docs" target="_blank" rel="noreferrer" aria-label="使用文档" icon={<BookOpen aria-hidden="true" className="size-4" />}>{collapsed ? null : '使用文档'}</AntButton></Tooltip>,
      <Tooltip key="logout" title={collapsed ? '退出登录' : undefined} placement="right"><AntButton type="text" aria-label="退出登录" icon={<LogoutOutlined aria-hidden="true" />} disabled={loggingOut} onClick={logout}>{collapsed ? null : '退出登录'}</AntButton></Tooltip>,
    ]}
    token={{ sider: { colorMenuBackground: '#0a1216', colorTextMenu: '#9aaeb3', colorTextMenuSelected: '#78d9d0', colorBgMenuItemSelected: '#16222a' }, header: { colorBgHeader: '#0a1216' } }}
    pageTitleRender={false}
    footerRender={false}
  ><main id="main-content" tabIndex={-1} className="mx-auto w-full max-w-7xl px-4 py-6 md:px-8 md:py-8"><ErrorBox error={error} />{account.isError ? <ErrorBox error={account.error} retry={() => account.refetch()} /> : <Suspense fallback={<Loading />}><Outlet /></Suspense>}</main></ProLayout></div>
}
export function App() {
  const location = useLocation()
  const me = useQuery({ queryKey: ['me'], queryFn: ({ signal }) => api<User>('/me', { signal }), retry: false })
  if (me.isPending) return <div className="mx-auto max-w-4xl p-8"><Loading /></div>
  if (me.isError) return me.error instanceof ApiError && me.error.status === 401 ? <Navigate to="/login" state={{ from: location.pathname + location.hash }} replace /> : <div className="mx-auto max-w-xl p-8"><ErrorBox error={me.error} retry={() => me.refetch()} /></div>
  return <ConfigProvider theme={consoleTheme}><UserContext.Provider value={me.data}><Shell user={me.data} /></UserContext.Provider></ConfigProvider>
}
