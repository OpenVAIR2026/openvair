import type { ReactNode } from 'react'

// A deliberately small highlighter: the docs only show shell, config and short SDK snippets,
// so a few token rules per language read well without shipping a full grammar library.
const STRING = /"(?:[^"\\\n]|\\.)*"|'[^'\n]*'/
const HASH_COMMENT = /(?<=^|\s)#[^\n]*/
const NUMBER = /\b\d+(?:\.\d+)?\b/
const LITERAL = /\b(?:true|false|null|None|True|False)\b/

const shell: [string, RegExp][] = [
  ['comment', HASH_COMMENT],
  ['string', STRING],
  ['variable', /\$(?:env:)?\{?\w+\}?/],
  ['flag', /(?<=\s)--?[A-Za-z][\w-]*/],
  ['command', /(?<=^[ \t]*)[\w./-]+/],
]

const grammars: Record<string, [string, RegExp][]> = {
  bash: shell,
  sh: shell,
  shell,
  powershell: shell,
  json: [
    ['property', /"(?:[^"\\\n]|\\.)*"(?=\s*:)/],
    ['string', STRING],
    ['number', NUMBER],
    ['literal', LITERAL],
  ],
  toml: [
    ['comment', HASH_COMMENT],
    ['section', /^[ \t]*\[[^\]\n]+\]/],
    ['property', /(?<=^[ \t]*)[\w.-]+(?=[ \t]*=)/],
    ['string', STRING],
    ['number', NUMBER],
    ['literal', LITERAL],
  ],
  python: [
    ['comment', HASH_COMMENT],
    ['string', STRING],
    ['keyword', /\b(?:import|from|as|def|return|async|await|with|for|in|if|else|print)\b/],
    ['literal', LITERAL],
    ['call', /\b[A-Za-z_]\w*(?=\()/],
    ['number', NUMBER],
  ],
  javascript: [
    ['comment', /\/\/[^\n]*/],
    ['string', /`(?:[^`\\]|\\.)*`|"(?:[^"\\\n]|\\.)*"|'[^'\n]*'/],
    ['keyword', /\b(?:import|from|const|let|new|await|async|return|export|default|for|of)\b/],
    ['literal', LITERAL],
    ['call', /\b[A-Za-z_$][\w$]*(?=\()/],
    ['number', NUMBER],
  ],
}
grammars.js = grammars.javascript
grammars.ts = grammars.javascript
grammars.typescript = grammars.javascript
grammars.py = grammars.python

const compiled = new Map<string, RegExp>()
function patternFor(language: string) {
  const rules = grammars[language]
  if (!rules) return null
  let pattern = compiled.get(language)
  if (!pattern) {
    pattern = new RegExp(rules.map(([kind, rule]) => `(?<${kind}>${rule.source})`).join('|'), 'gm')
    compiled.set(language, pattern)
  }
  return pattern
}

export function highlight(code: string, language?: string): ReactNode {
  const pattern = language ? patternFor(language) : null
  if (!pattern) return code
  const parts: ReactNode[] = []
  let last = 0
  for (const match of code.matchAll(pattern)) {
    const kind = Object.entries(match.groups ?? {}).find(([, value]) => value !== undefined)?.[0]
    if (!kind || !match[0]) continue
    if (match.index > last) parts.push(code.slice(last, match.index))
    parts.push(<span key={match.index} className={`tok-${kind}`}>{match[0]}</span>)
    last = match.index + match[0].length
  }
  parts.push(code.slice(last))
  return parts
}
