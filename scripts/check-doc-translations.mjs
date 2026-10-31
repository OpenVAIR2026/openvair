#!/usr/bin/env node
// Every Chinese doc needs an English twin and vice versa: console docs live in
// web/src/docs/content/{,en/}, repository docs are `X.md` next to `X.en.md`.
import { existsSync, readdirSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

const repo = fileURLToPath(new URL('..', import.meta.url))
const missing = []

const content = `${repo}web/src/docs/content`
const md = dir => readdirSync(dir).filter(file => file.endsWith('.md'))
const zh = new Set(md(content))
const en = new Set(md(`${content}/en`))
for (const file of zh) if (!en.has(file)) missing.push(`web/src/docs/content/en/${file}`)
for (const file of en) if (!zh.has(file)) missing.push(`web/src/docs/content/${file}`)

const repoDocs = ['README.md', 'CONTRIBUTING.md', ...md(`${repo}docs`).filter(file => !file.endsWith('.en.md')).map(file => `docs/${file}`)]
for (const file of repoDocs) {
  const twin = file.replace(/\.md$/, '.en.md')
  if (!existsSync(`${repo}${twin}`)) missing.push(twin)
}
for (const file of md(`${repo}docs`).filter(file => file.endsWith('.en.md'))) {
  if (!existsSync(`${repo}docs/${file.replace(/\.en\.md$/, '.md')}`)) missing.push(`docs/${file.replace(/\.en\.md$/, '.md')}`)
}

if (missing.length) {
  console.error(`Missing translations:\n${missing.map(file => `  ${file}`).join('\n')}`)
  process.exit(1)
}
console.log(`Doc translations complete: ${zh.size} console pages, ${repoDocs.length} repository docs.`)
