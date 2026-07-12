import { slug } from 'github-slugger'

const SCHEME = /^[a-z][a-z\d+.-]*:/i
const SAFE_SCHEME = /^(?:https?|mailto):/i
const HEADING_PREFIX = 'readme-'

export function safeMarkdownUrl(url: string) {
  if (url.startsWith('#')) return readmeFragment(url.slice(1))
  if (SAFE_SCHEME.test(url)) return url
  return ''
}

export function resolveReadmeUrl(
  url: string,
  context: { owner: string; readmePath: string; repo: string },
) {
  const safeUrl = safeMarkdownUrl(url)
  if (safeUrl) return safeUrl
  if (!url || SCHEME.test(url) || url.startsWith('//') || url.includes('?')) {
    return ''
  }

  const hashIndex = url.indexOf('#')
  const relativePath = hashIndex === -1 ? url : url.slice(0, hashIndex)
  const fragment =
    hashIndex === -1 ? '' : readmeFragment(url.slice(hashIndex + 1))
  const parts = relativePath.startsWith('/')
    ? []
    : context.readmePath.replace(/^\/+/, '').split('/').slice(0, -1)

  for (const encodedPart of relativePath.split('/')) {
    let part: string
    try {
      part = decodeURIComponent(encodedPart)
    } catch {
      return ''
    }
    if (part.includes('/')) return ''
    if (!part || part === '.') continue
    if (part === '..') {
      if (parts.length === 0) return ''
      parts.pop()
    } else {
      parts.push(part)
    }
  }

  if (parts.length === 0) return ''
  const repositoryPath = `/repos/${encodeURIComponent(context.owner)}/${encodeURIComponent(context.repo)}`
  return `${repositoryPath}?file=${encodeURIComponent(parts.join('/'))}${fragment}`
}

function readmeFragment(value: string) {
  if (/^user-content-fn(?:ref)?-/i.test(value)) return `#${value}`
  try {
    return `#${HEADING_PREFIX}${slug(decodeURIComponent(value))}`
  } catch {
    return ''
  }
}
