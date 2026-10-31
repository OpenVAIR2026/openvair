import { useEffect, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Download } from 'lucide-react'
import { Link } from 'react-router-dom'
import { ErrorBox, Loading } from '../components/common'
import { SiteHeader } from '../components/site-header'
import { Button } from '../components/ui/button'
import './home.css'
import './download.css'

interface Installer { platform: string; label: string; name: string; size: number; sha256: string; url: string }
interface Manifest { version: string; published_at: string; files: Installer[] }

/** Published by the Desktop workflow; absent until the first release. */
async function loadManifest(signal: AbortSignal): Promise<Manifest | null> {
  const response = await fetch('/downloads/desktop/latest.json', { signal, cache: 'no-cache' })
  if (response.status === 404) return null
  if (!response.ok) throw new Error('无法读取下载信息')
  return response.json()
}

type System = 'macos' | 'windows' | 'linux' | 'other'

function detectSystem(): System {
  const agent = navigator.userAgent
  if (/Windows/.test(agent)) return 'windows'
  if (/Mac OS X|Macintosh/.test(agent) && !/iPhone|iPad/.test(agent)) return 'macos'
  if (/Linux/.test(agent) && !/Android/.test(agent)) return 'linux'
  return 'other'
}

/** Chromium can tell Apple silicon from Intel; other browsers default to Apple silicon. */
function useMacArchitecture() {
  const [arch, setArch] = useState<'arm64' | 'x64'>('arm64')
  useEffect(() => {
    const data = (navigator as Navigator & { userAgentData?: { getHighEntropyValues?: (hints: string[]) => Promise<{ architecture?: string }> } }).userAgentData
    data?.getHighEntropyValues?.(['architecture']).then(value => { if (value.architecture === 'x86') setArch('x64') }).catch(() => undefined)
  }, [])
  return arch
}

const SYSTEMS: { id: Exclude<System, 'other'>; name: string; notes: string[] }[] = [
  { id: 'macos', name: 'macOS', notes: ['打开 .dmg，把 VAIR Desktop 拖进“应用程序”。', '首次打开被拦下时，到“系统设置 › 隐私与安全性”点“仍要打开”。macOS 14 及更早也可以右键点它选“打开”。测试版未经 Apple 公证。', '提示已损坏时，在终端运行下面的命令后再打开：`xattr -cr "/Applications/VAIR Desktop.app"`'] },
  { id: 'windows', name: 'Windows', notes: ['运行安装程序。', 'SmartScreen 提示时点“更多信息 › 仍要运行”。测试版未做代码签名。'] },
  { id: 'linux', name: 'Linux', notes: ['Debian / Ubuntu `sudo apt install ./文件名.deb`', 'Fedora / RHEL `sudo dnf install ./文件名.rpm`'] },
]

/** Names inside a system's group, where the system name would only repeat. */
const VARIANT: Record<string, string> = {
  'macos-arm64': 'Apple 芯片',
  'macos-x64': 'Intel 芯片',
  'windows-x64': '安装程序 .exe',
  'windows-x64-msi': '安装包 .msi',
  'linux-deb': '.deb 包',
  'linux-rpm': '.rpm 包',
}

const megabytes = (size: number) => `${(size / 1048576).toFixed(1)} MB`

export function DownloadPage() {
  const manifest = useQuery({ queryKey: ['desktop-manifest'], queryFn: ({ signal }) => loadManifest(signal), retry: false })
  const system = detectSystem()
  const arch = useMacArchitecture()
  const files = manifest.data?.files ?? []
  const preferred = system === 'windows' ? 'windows-x64' : system === 'linux' ? 'linux-deb' : system === 'macos' ? `macos-${arch}` : ''
  const primary = files.find(file => file.platform === preferred)
  // The detected system comes first; the others follow in their usual order.
  const systems = [...SYSTEMS].sort((a, b) => Number(b.id === system) - Number(a.id === system))

  return <div className="home-page">
    <SiteHeader />
    <main>
      <section className="dl-hero home-container" aria-labelledby="download-title">
        <div className="dl-intro">
          <h1 id="download-title">VAIR Desktop</h1>
          <p className="dl-lead">找出本机装了的 Codex、Claude Code、Gemini CLI 等 Agent，选好 Key 和模型就写入配置。写之前先备份，写完自动测一次连通。</p>
          <p className="dl-lead">登录 VAIR 帐号，或者直接粘贴别人发给你的 VAIR Key，都可以用。</p>
          {manifest.isPending ? <Loading />
            : manifest.isError ? <ErrorBox error={manifest.error} retry={() => manifest.refetch()} />
            : !manifest.data ? <p className="dl-meta">客户端即将发布，敬请期待。</p>
            : <div className="home-actions">
              {primary
                ? <Button asChild size="lg" className="home-primary-action"><a href={primary.url} download><Download aria-hidden="true" />下载 {primary.label}</a></Button>
                : <p className="dl-meta">VAIR Desktop 支持 macOS、Windows 和 Linux，请在电脑上下载。</p>}
              <span className="dl-meta">版本 {manifest.data.version}，测试版</span>
            </div>}
        </div>
        <figure className="dl-shot">
          <img src="/docs-img/desktop-agents-home.webp" width={1920} height={1240} alt="VAIR Desktop 的本机 Agent 页：六个 Agent 已接入 VAIR，转到中间的是 Claude Code，每个都显示所用的模型，可以测试连通或更换 Key。" />
        </figure>
      </section>

      {manifest.data && <section className="dl-files home-container" aria-labelledby="files-title">
        <h2 id="files-title">全部安装包</h2>
        <div className="dl-systems">
          {systems.map(({ id, name, notes }) => {
            const own = files.filter(file => file.platform.startsWith(`${id}-`))
            if (!own.length) return null
            return <section key={id} className="dl-system" aria-labelledby={`system-${id}`}>
              <h3 id={`system-${id}`}>{name}</h3>
              <ul className="dl-packages">
                {own.map(file => <li key={file.platform}>
                  <a href={file.url} download aria-label={`下载 ${file.label}`}>
                    <span>{VARIANT[file.platform] ?? file.label}</span>
                    <small title={`SHA-256 ${file.sha256}`}>{megabytes(file.size)}</small>
                    <Download aria-hidden="true" size={16} />
                  </a>
                </li>)}
              </ul>
              <ol className="dl-notes">{notes.map(note => <li key={note}>{note.split('`').map((part, index) => index % 2 ? <code key={index}>{part}</code> : part)}</li>)}</ol>
            </section>
          })}
        </div>
        <p className="dl-footnote">鼠标停在文件大小上可以看到 SHA-256 校验值。装好后在客户端点“在浏览器中登录”，到本站确认授权码即可。手动设置见 <Link to="/docs/desktop">桌面客户端文档</Link>。</p>
      </section>}
    </main>
  </div>
}
