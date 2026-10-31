import { useLocation } from 'react-router-dom'

/** Chinese lives at `/`, English under `/en`; the URL is the only source of the current language. */
export type Lang = 'zh' | 'en'

export const LANG_KEY = 'openvair.site.lang'

export const langOf = (pathname: string): Lang => /^\/en(\/|$)/.test(pathname) ? 'en' : 'zh'

/** The same page in the given language. Anchors are dropped: headings differ between languages. */
export function pathIn(lang: Lang, pathname: string) {
  const bare = pathname.replace(/^\/en(?=\/|$)/, '') || '/'
  return lang === 'en' ? (bare === '/' ? '/en' : `/en${bare}`) : bare
}

const zh = {
  htmlLang: 'zh-CN',
  siteTitle: 'OpenVAIR · 自部署的 AI 网关',
  skip: '跳到主要内容',
  nav: { home: '首页', deploy: '部署', docs: '文档', download: '下载', label: '站点导航', homeLabel: 'OpenVAIR 首页', openMenu: '打开菜单', closeMenu: '关闭菜单', switchLabel: '切换语言' },
  footer: {
    tagline: '面向小团队和课题组的自部署 AI 网关。',
    product: '产品', deploy: '部署', desktop: '桌面客户端', comparison: '同类对比',
    docs: '文档', quickstart: '快速开始', troubleshooting: '问题排查',
    community: '社区', issues: '反馈问题', security: '安全报告',
    legal: '服务端和控制台使用 AGPL-3.0 许可证，桌面客户端使用 Apache-2.0 许可证。OpenVAIR 不提供模型额度，调用费用由你接入的模型供应商结算。',
  },
  copy: { terminal: '终端', copy: '复制', copied: '已复制', copyCommand: '复制命令' },
  md: {
    callouts: { note: '说明', tip: '提示', important: '重要', warning: '注意', caution: '警告' },
    table: '表格，可横向滚动',
    consoleLink: '这是 OpenVAIR 控制台中的页面，部署后在你自己的服务器上打开',
    toc: '本页内容', tocLabel: '本页目录',
  },
  home: {
    title: ['一条命令完成部署，', '即可使用'],
    titleEm: '填入 Key',
    lead: '团队共用的自部署 AI 网关，统一管理 Key、限额和用量。',
    deploy: '部署到服务器', download: '下载桌面客户端',
    simple: {
      kicker: '设计理念：极简，易用',
      items: [
        { unit: '条命令', label: '部署到服务器', chip: 'install.sh' },
        { unit: '把 Key', label: '分给每位成员', chip: '独立限额' },
        { unit: '次点击', label: '客户端接入', chip: '「接入」' },
      ],
    },
    route: {
      kicker: '统一入口', title: ['所有工具接入同一个地址，', '渠道故障自动切换'],
      label: 'Claude Code、Codex、Gemini CLI 和 SDK 只连接 OpenVAIR，OpenVAIR 按优先级转发到官方 API、中转站或内网模型；中转站故障时自动改走其他渠道。',
      tools: ['Claude Code', 'Codex', 'Gemini CLI', '实验脚本 · SDK'],
      channels: ['官方 API', '中转站', '内网模型'],
      hub: '鉴权 · 限额 · 切换', down: '429 限流，已绕过', ok: '正常',
    },
    audiences: {
      kicker: '适用对象', title: '小团队与科研课题组',
      items: [
        { title: '小团队', anchor: '小团队', points: ['每人一个 Key', '上游 Key 不外泄', '按成员统计费用'] },
        { title: '科研课题组', anchor: '科研课题组', points: ['按学生或课题设上限', '内网模型优先', '不保存对话内容'] },
      ],
      more: (title: string) => `${title}的用法`,
    },
    gallery: {
      kicker: '产品界面', title: '渠道、Key、监控、用量', label: '界面截图',
      shots: {
        desktop: { label: '桌面客户端', caption: '各 Agent 的 Key、模型和当日用量。' },
        keys: { label: 'API Key', caption: '每人独立的 Key，可设额度和可用模型。' },
        channels: { label: '渠道', caption: '官方 API、中转站和内网服务，按优先级切换。' },
        monitor: { label: '监控', caption: '定时检查可用性，持续失败时降级。' },
        usage: { label: '用量', caption: '按 Key、模型和渠道统计 Token 与费用。' },
      },
    },
    cta: {
      title: '几分钟完成部署',
      body: '一台装有 Docker 的 Linux 服务器，x86_64 或 arm64。',
      guide: '阅读部署指南', quickstart: '快速开始',
    },
  },
  deploy: {
    title: '部署', kicker: '部署', heading: '部署到你自己的服务器',
    lead: '一条命令安装，浏览器中创建管理员。再次运行同一命令即可升级。',
    facts: [
      ['几分钟完成安装', '拉取预构建镜像，密钥自动生成'],
      ['三个容器', 'PostgreSQL、Redis 和 OpenVAIR'],
      ['x86_64 / arm64', '64 位 Linux，Docker Compose v2'],
    ],
    nextTitle: '部署完成后', next: '接入第一个渠道并创建第一个 Key',
  },
  docs: {
    title: '文档', kicker: '文档', heading: 'OpenVAIR 文档',
    lead: ['与控制台内的「文档」相同。安装见', '部署', '，部署后从', '快速开始', '读起。'],
    sidebar: '文档目录', home: '文档首页',
    pager: '上一篇和下一篇', previous: '上一篇', next: '下一篇',
    missingTitle: '页面不存在', missing: '该文档可能已更名或合并，请通过左侧目录查找。', back: '返回文档首页',
    about: '了解 OpenVAIR',
    useCases: ['适用场景', '小团队、课题组、个人和应用开发者的用法。'],
    comparison: ['与同类项目对比', '做什么、不做什么，以及何时选其他项目。'],
  },
  download: {
    title: '下载桌面客户端',
    heading: ['填入 Key，', '完成本机 Agent 配置'],
    lead: '填入服务器地址和 Key，选择模型后点击「接入」。断开时恢复原有配置。',
    get: (name: string) => `下载 ${name} 版`, latest: '下载最新版', source: '从源码构建',
    agents: '支持一键接入的 Agent',
    shotAlt: 'OpenVAIR Desktop 的本机 Agent 页：每个 Agent 使用的 Key 和模型',
    files: { mac: '.dmg · Apple 芯片和 Intel', windows: '.msi / .exe 安装程序', linux: '.deb / .rpm / AppImage' },
    current: '当前系统',
    note: ['安装包发布在 ', '。开源版不自动更新，新版本链接见「关于」页。'],
    install: '安装', installSection: '下载和安装',
    nextTitle: '安装完成后', next: '首次启动、添加 Key 与接入 Agent',
  },
}

export type Strings = typeof zh

const en: Strings = {
  htmlLang: 'en',
  siteTitle: 'OpenVAIR · Self-hosted AI gateway',
  skip: 'Skip to main content',
  nav: { home: 'Home', deploy: 'Deploy', docs: 'Docs', download: 'Download', label: 'Site navigation', homeLabel: 'OpenVAIR home', openMenu: 'Open menu', closeMenu: 'Close menu', switchLabel: 'Change language' },
  footer: {
    tagline: 'The self-hosted AI gateway for small teams and research groups.',
    product: 'Product', deploy: 'Deploy', desktop: 'Desktop app', comparison: 'Comparison',
    docs: 'Docs', quickstart: 'Quick start', troubleshooting: 'Troubleshooting',
    community: 'Community', issues: 'Report an issue', security: 'Security',
    legal: 'The server and console are licensed under AGPL-3.0, the desktop app under Apache-2.0. OpenVAIR does not sell model usage; you pay the providers you connect directly.',
  },
  copy: { terminal: 'Terminal', copy: 'Copy', copied: 'Copied', copyCommand: 'Copy command' },
  md: {
    callouts: { note: 'Note', tip: 'Tip', important: 'Important', warning: 'Warning', caution: 'Caution' },
    table: 'Table, scrolls horizontally',
    consoleLink: 'This page is part of the OpenVAIR console. Open it on your own server after deploying.',
    toc: 'On this page', tocLabel: 'On this page',
  },
  home: {
    title: ['One-command deploy.', 'and go.'],
    titleEm: 'Paste a key',
    lead: 'A self-hosted AI gateway for teams: keys, limits and usage.',
    deploy: 'Deploy on your server', download: 'Get the desktop app',
    simple: {
      kicker: 'Design: simple, easy',
      items: [
        { unit: ' command', label: 'to deploy', chip: 'install.sh' },
        { unit: ' key', label: 'per member', chip: 'own limits' },
        { unit: ' click', label: 'in the desktop app', chip: 'Connect' },
      ],
    },
    route: {
      kicker: 'One endpoint', title: ['Every tool, one address.', 'Failed channels are skipped.'],
      label: 'Claude Code, Codex, Gemini CLI and SDKs connect only to OpenVAIR, which forwards requests by priority to an official API, a relay service or an internal model server. When the relay fails, requests move to another channel automatically.',
      tools: ['Claude Code', 'Codex', 'Gemini CLI', 'Scripts · SDKs'],
      channels: ['Official API', 'Relay service', 'Internal models'],
      hub: 'Auth · Limits · Failover', down: '429 rate limited, skipped', ok: 'Healthy',
    },
    audiences: {
      kicker: 'Who it is for', title: 'Small teams and research groups',
      items: [
        { title: 'Small teams', anchor: 'small-teams', points: ['One key per person', 'Upstream keys stay on the server', 'Spend per member'] },
        { title: 'Research groups', anchor: 'research-groups', points: ['Caps per student or project', 'Internal models first', 'No conversations stored'] },
      ],
      more: (title: string) => `How ${title.toLowerCase()} use it`,
    },
    gallery: {
      kicker: 'The product', title: 'Channels, keys, monitoring, usage', label: 'Screenshots',
      shots: {
        desktop: { label: 'Desktop app', caption: 'Key, model and today’s usage for each agent.' },
        keys: { label: 'API keys', caption: 'A key per member, with quota and allowed models.' },
        channels: { label: 'Channels', caption: 'Official APIs, relays and internal servers, failover by priority.' },
        monitor: { label: 'Monitoring', caption: 'Scheduled checks; failing channels are demoted.' },
        usage: { label: 'Usage', caption: 'Tokens and spend by key, model and channel.' },
      },
    },
    cta: {
      title: 'Running in minutes',
      body: 'A Linux server with Docker, x86_64 or arm64.',
      guide: 'Read the deployment guide', quickstart: 'Quick start',
    },
  },
  deploy: {
    title: 'Deploy', kicker: 'Deploy', heading: 'Run it on your own server',
    lead: 'One command to install, the admin is created in the browser. Run it again to upgrade.',
    facts: [
      ['Minutes to install', 'Prebuilt images, secrets generated for you'],
      ['Three containers', 'PostgreSQL, Redis and OpenVAIR'],
      ['x86_64 / arm64', '64-bit Linux, Docker Compose v2'],
    ],
    nextTitle: 'Deployed?', next: 'Add your first channel and issue your first key',
  },
  docs: {
    title: 'Docs', kicker: 'Docs', heading: 'OpenVAIR docs',
    lead: ['The same docs as in the console. To install, see ', 'Deploy', '; then start with ', 'Quick start', '.'],
    sidebar: 'Contents', home: 'Docs home',
    pager: 'Previous and next page', previous: 'Previous', next: 'Next',
    missingTitle: 'Page not found', missing: 'This page may have been renamed or merged. Try the contents on the left.', back: 'Back to docs home',
    about: 'About OpenVAIR',
    useCases: ['Use cases', 'How teams, research groups, individuals and app developers use it.'],
    comparison: ['Compared with similar projects', 'What it does, what it leaves out, and when to pick another project.'],
  },
  download: {
    title: 'Download the desktop app',
    heading: ['Paste a key.', 'Your local agents are ready.'],
    lead: 'Enter the server address and your key, pick a model and click Connect. Disconnecting restores your original config.',
    get: (name: string) => `Download for ${name}`, latest: 'Download the latest release', source: 'Build from source',
    agents: 'Agents with one-click setup',
    shotAlt: 'The local agents page in OpenVAIR Desktop: the key and model each agent uses',
    files: { mac: '.dmg · Apple silicon and Intel', windows: '.msi / .exe installer', linux: '.deb / .rpm / AppImage' },
    current: 'Your system',
    note: ['Installers are on ', '. No auto-update; the About page links to new releases.'],
    install: 'Install', installSection: 'Download and install',
    nextTitle: 'Installed?', next: 'First launch, adding a key and connecting agents',
  },
}

export const strings: Record<Lang, Strings> = { zh, en }

export function useLang() {
  const { pathname } = useLocation()
  const lang = langOf(pathname)
  /** Prefixes a site path with `/en` when the page is English. */
  const to = (path: string) => lang === 'en' ? (path === '/' ? '/en' : `/en${path}`) : path
  return { lang, t: strings[lang], to }
}
