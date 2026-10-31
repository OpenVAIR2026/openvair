export function isConcreteModel(model: string) {
  return model.length > 0 && model.trim() === model && new TextEncoder().encode(model).length <= 256 && !/[*\x00-\x1f\x7f]/.test(model)
}
export function isModelRule(rule: string) {
  return isConcreteModel(rule.endsWith('*') ? rule.slice(0, -1) : rule)
}
export function matchesModel(rule: string, model: string) {
  return isConcreteModel(model) && isModelRule(rule) && (rule.endsWith('*') ? model.startsWith(rule.slice(0, -1)) : model === rule)
}
/** Whether some model could satisfy both rules, e.g. `gpt-*` and `gpt-4*`. */
export function rulesOverlap(a: string, b: string) {
  const prefixA = a.endsWith('*') ? a.slice(0, -1) : null
  const prefixB = b.endsWith('*') ? b.slice(0, -1) : null
  if (prefixA != null && prefixB != null) return prefixA.startsWith(prefixB) || prefixB.startsWith(prefixA)
  if (prefixA != null) return b.startsWith(prefixA)
  if (prefixB != null) return a.startsWith(prefixB)
  return a === b
}
export function modelScopeLabel(rules: string[], mode: string) {
  if (mode === 'passthrough') return '全部转发'
  if (mode === 'all') return '全部模型'
  const prefixes = rules.filter(rule => rule.endsWith('*')).length
  const exact = rules.length - prefixes
  return [exact ? `${exact} 个指定模型` : '', prefixes ? `${prefixes} 条前缀规则` : ''].filter(Boolean).join(' + ') || '未配置规则'
}

export function splitModelTokens(value: string) {
  return value.split(/[,，\n]/).map(item => item.trim()).filter(Boolean)
}
