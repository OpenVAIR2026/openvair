import { mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { defineConfig, type Plugin } from 'vite'
import react from '@vitejs/plugin-react'

const repo = fileURLToPath(new URL('..', import.meta.url))
/** Where the built site is published; used for canonical and og:url links. */
const ORIGIN = 'https://openvair.com'

const english = {
  title: 'OpenVAIR · Self-hosted AI gateway',
  description: 'OpenVAIR is a self-hosted AI gateway for small teams and research groups. Deploy it with one command; Claude Code, Codex and Gemini CLI connect to it with one key per person, using your own model channels.',
}

/**
 * Every route gets its own index.html so the build can be served by any static host without rewrites.
 * Chinese pages live at the root and English pages under /en/, each with its own lang, title and description.
 */
function staticRoutes(): Plugin {
  let outDir = ''
  return {
    name: 'static-routes',
    apply: 'build',
    configResolved: config => { outDir = config.build.outDir },
    closeBundle() {
      const docs = readdirSync(`${repo}web/src/docs/content`).filter(file => file.endsWith('.md') && file !== 'index.md').map(file => file.slice(0, -3))
      const routes = ['deploy', 'download', 'docs', 'docs/use-cases', 'docs/comparison', ...docs.map(slug => `docs/${slug}`)]
      const zh = readFileSync(`${outDir}/index.html`, 'utf8')
      const en = zh
        .replace('<html lang="zh-CN">', '<html lang="en">')
        .replace(/<title>[^<]*<\/title>/, `<title>${english.title}</title>`)
        .replace(/<meta name="description" content="[^"]*" \/>/, `<meta name="description" content="${english.description}" />`)
      if (en === zh) throw new Error('index.html no longer matches the English rewrite')
      // Each page names its canonical URL and its other-language twin.
      const withUrls = (html: string, route: string, lang: 'zh' | 'en') => {
        const path = route ? `/${route}/` : '/'
        const zhUrl = `${ORIGIN}${path}`
        const enUrl = `${ORIGIN}/en${path}`
        const self = lang === 'en' ? enUrl : zhUrl
        const tags = [
          `<link rel="canonical" href="${self}" />`,
          `<meta property="og:url" content="${self}" />`,
          `<link rel="alternate" hreflang="zh-CN" href="${zhUrl}" />`,
          `<link rel="alternate" hreflang="en" href="${enUrl}" />`,
          `<link rel="alternate" hreflang="x-default" href="${zhUrl}" />`,
        ].join('\n    ')
        return html.replace('</head>', `    ${tags}\n  </head>`)
      }
      const write = (route: string, html: string, lang: 'zh' | 'en') => {
        const dir = lang === 'en' ? `en/${route}`.replace(/\/$/, '') : route
        mkdirSync(`${outDir}/${dir}`, { recursive: true })
        writeFileSync(`${outDir}/${dir}/index.html`, withUrls(html, route, lang))
      }
      write('', zh, 'zh')
      for (const route of routes) write(route, zh, 'zh')
      write('', en, 'en')
      for (const route of routes) write(route, en, 'en')
      // Hosts that serve the nearest 404.html per directory get the English shell under /en/.
      writeFileSync(`${outDir}/404.html`, zh)
      writeFileSync(`${outDir}/en/404.html`, en)
    },
  }
}

export default defineConfig({
  plugins: [react(), staticRoutes()],
  // Screenshots come straight from the console's docs so both sites show the same images.
  publicDir: `${repo}web/public`,
  resolve: { dedupe: ['react', 'react-dom'] },
  server: { fs: { allow: [repo] } },
})
