import { useCallback, useEffect, useState } from 'react'
import type { DocLang } from './manifest'

const KEY = 'openvair.docs.lang'

function initialLang(): DocLang {
  try {
    const saved = localStorage.getItem(KEY)
    if (saved === 'zh' || saved === 'en') return saved
  } catch { /* storage can be blocked; fall back to the browser language */ }
  return /^zh\b/i.test(navigator.language) ? 'zh' : 'en'
}

/** Only the docs page is bilingual: it follows the browser until the reader picks a language, then remembers it. */
export function useDocsLang() {
  const [lang, setLang] = useState(initialLang)
  const choose = useCallback((next: DocLang) => {
    setLang(next)
    try { localStorage.setItem(KEY, next) } catch { /* the choice still holds for this visit */ }
  }, [])
  useEffect(() => {
    const root = document.documentElement
    const previous = root.lang
    root.lang = lang === 'en' ? 'en' : 'zh-CN'
    return () => { root.lang = previous }
  }, [lang])
  return [lang, choose] as const
}

const zh = {
  docs: '文档',
  callouts: { note: '说明', tip: '提示', important: '重要', warning: '注意', caution: '警告' },
  groups: {
    start: '了解 {product}，跑通第一次请求',
    tools: 'Claude Code、Codex、Gemini CLI 和 SDK',
    channels: '接入上游、映射模型、故障切换',
    keys: '分发 Key，限制模型、频率和费用',
    monitoring: '可用性、服务体检、真伪检测和用量',
    account: '登录设备、邮件提醒和数据隐私',
    help: '按报错查原因，常见疑问',
  } as Record<string, string>,
  popular: ['429', 'Claude Code', '首字超时', '限额', 'invalid_api_key'],
  copy: '复制', copied: '已复制', copyCode: '复制代码', copyFailed: '复制失败',
  zoom: '放大查看：',
  table: '表格，可横向滚动',
  search: '搜索文档', searchPlaceholder: '搜索文档、错误码或环境变量', searchResults: '搜索结果',
  common: '常用：', popularLabel: '大家常搜：',
  noResults: (query: string) => `没有找到「${query}」。换个关键词试试，或查看`, faq: '常见问题',
  select: '选择', open: '打开', close: '关闭',
  sidebar: '文档目录',
  flow: { label: (product: string) => `请求经过 ${product} 的路径：渠道 A 限流时，请求自动改走渠道 B`, tools: '你的工具', hub: '鉴权 · 限额 · 路由', a: '渠道 A', b: '渠道 B', limited: '429 限流', healthy: '健康 · 接管请求', caption: '渠道 A 限流时，请求自动改走渠道 B，客户端无感知。' },
  hero: { eyebrow: (product: string) => `${product} 文档`, title: '一个地址，一把 Key，', titleEm: '接入你所有的模型渠道', lead: '接入渠道、发放 Key、配置 Claude Code 和 Codex，以及请求出错时怎么查。', quickstart: '5 分钟快速开始' },
  pages: (count: number) => `${count} 篇`,
  skip: '跳到主要内容', openMenu: '打开目录', closeMenu: '关闭目录', home: '首页',
  siteNav: '站点导航', verify: '模型验证', desktop: '桌面客户端', login: '登录', console: '进入控制台', start: '免费开始',
  toc: '本页内容', tocLabel: '本页目录', pager: '上一篇和下一篇', previous: '上一篇', next: '下一篇',
  feedback: ['没找到答案？先看看', '问题排查', '，或在控制台的「操作日志」和「使用分析」里查看具体记录。'],
  missingTitle: '页面不存在', missing: ['这篇文档可能已经改名或合并。可以从左侧目录找，或者', '搜索文档', '。'], back: '返回文档首页',
  language: '文档语言',
}

const en: typeof zh = {
  docs: 'Docs',
  callouts: { note: 'Note', tip: 'Tip', important: 'Important', warning: 'Warning', caution: 'Caution' },
  groups: {
    start: 'Learn {product} and send your first request',
    tools: 'Claude Code, Codex, Gemini CLI and SDKs',
    channels: 'Upstreams, model mapping and failover',
    keys: 'Issue keys; limit models, rate and spend',
    monitoring: 'Availability, checkups, authenticity and usage',
    account: 'Devices, alert emails and data privacy',
    help: 'Look up errors, common questions',
  },
  popular: ['429', 'Claude Code', 'first token', 'limits', 'invalid_api_key'],
  copy: 'Copy', copied: 'Copied', copyCode: 'Copy code', copyFailed: 'Copy failed',
  zoom: 'Enlarge: ',
  table: 'Table, scrolls horizontally',
  search: 'Search docs', searchPlaceholder: 'Search docs, error codes or env vars', searchResults: 'Search results',
  common: 'Popular: ', popularLabel: 'Popular searches: ',
  noResults: (query: string) => `Nothing found for “${query}”. Try another term, or see the `, faq: 'FAQ',
  select: 'select', open: 'open', close: 'close',
  sidebar: 'Contents',
  flow: { label: (product: string) => `How a request passes through ${product}: when channel A is rate limited, it moves to channel B`, tools: 'Your tools', hub: 'Auth · Limits · Routing', a: 'Channel A', b: 'Channel B', limited: '429 rate limited', healthy: 'Healthy · takes over', caption: 'When channel A is rate limited, requests move to channel B. Clients never notice.' },
  hero: { eyebrow: (product: string) => `${product} docs`, title: 'One address, one key, ', titleEm: 'all your model channels', lead: 'Add channels, issue keys, set up Claude Code and Codex, and find out what went wrong when a request fails.', quickstart: '5-minute quick start' },
  pages: (count: number) => `${count} ${count === 1 ? 'page' : 'pages'}`,
  skip: 'Skip to main content', openMenu: 'Open contents', closeMenu: 'Close contents', home: 'Home',
  siteNav: 'Site navigation', verify: 'Model check', desktop: 'Desktop app', login: 'Sign in', console: 'Open console', start: 'Start free',
  toc: 'On this page', tocLabel: 'On this page', pager: 'Previous and next page', previous: 'Previous', next: 'Next',
  feedback: ['Didn’t find an answer? Try ', 'Troubleshooting', ', or look up the exact records under 操作日志 (activity log) and 使用分析 (usage analytics) in the console.'],
  missingTitle: 'Page not found', missing: ['This page may have been renamed or merged. Try the contents on the left, or ', 'search the docs', '.'], back: 'Back to docs home',
  language: 'Docs language',
}

export const docsStrings: Record<DocLang, typeof zh> = { zh, en }
