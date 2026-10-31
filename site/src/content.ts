// The site renders the repository's markdown directly: usage docs come from the console's manifest,
// and the pages about choosing and deploying OpenVAIR come from docs/ (`*.en.md` for English).
import { docs, type DocPage } from '../../web/src/docs/manifest'
import useCasesZh from '../../docs/use-cases.md?raw'
import useCasesEn from '../../docs/use-cases.en.md?raw'
import comparisonZh from '../../docs/comparison.md?raw'
import comparisonEn from '../../docs/comparison.en.md?raw'
import selfHostingZh from '../../docs/self-hosting.md?raw'
import selfHostingEn from '../../docs/self-hosting.en.md?raw'
import { strings, type Lang } from './i18n'

export type { DocPage }

export { GITHUB, INSTALL, RELEASES } from './links'

/**
 * Files in docs/ may open with a language switch line (for GitHub readers) and a `# Title` line;
 * the site has its own switcher and shows the title in its own header instead.
 */
const withoutTitle = (body: string) => body.replace(/^\[中文\].*\n+/, '').replace(/^\[English\].*\n+/, '').replace(/^# .+\n+/, '')

const repoDocs = {
  zh: { useCases: useCasesZh, comparison: comparisonZh, selfHosting: selfHostingZh },
  en: { useCases: useCasesEn, comparison: comparisonEn, selfHosting: selfHostingEn },
}

function build(lang: Lang) {
  const t = strings[lang].docs
  const source = repoDocs[lang]
  const docGroups: { id: string; group: string; pages: DocPage[] }[] = [
    { id: 'about', group: t.about, pages: [
      { slug: 'use-cases', title: t.useCases[0], description: t.useCases[1], group: t.about, groupId: 'about', body: withoutTitle(source.useCases) },
      { slug: 'comparison', title: t.comparison[0], description: t.comparison[1], group: t.about, groupId: 'about', body: withoutTitle(source.comparison) },
    ] },
    // The console's welcome page talks about "this instance", so the site starts from its own docs home.
    ...docs[lang].docGroups.map(({ id, group, pages }) => ({ id, group, pages: pages.filter(page => page.slug) })),
  ]
  return { docGroups, docPages: docGroups.flatMap(({ pages }) => pages), deployDoc: withoutTitle(source.selfHosting) }
}

export const content: Record<Lang, ReturnType<typeof build>> = { zh: build('zh'), en: build('en') }

/** One `## heading` section of a page, without the heading, so a page can quote it instead of copying it. */
export function sectionOf(body: string, heading: string) {
  const start = body.indexOf(`\n## ${heading}\n`)
  if (start < 0) throw new Error(`missing section "${heading}"`)
  const rest = body.slice(start + heading.length + 5)
  const end = rest.search(/\n## /)
  return end < 0 ? rest : rest.slice(0, end)
}
