/**
 * Anchor paths give up their leading directories first, because the filename
 * is the part that identifies the code under discussion.
 */
export function anchorPathLabel(path: string, maxLength = 34) {
  const normalized = path.replace(/^\/+/, '')
  if (normalized.length <= maxLength) return normalized

  const segments = normalized.split('/')
  let label = segments.pop() ?? normalized
  while (segments.length > 0) {
    const candidate = `${segments[segments.length - 1]}/${label}`
    if (candidate.length + 2 > maxLength) break
    label = candidate
    segments.pop()
  }
  return `…/${label}`
}
