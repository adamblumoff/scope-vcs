import { slug } from 'github-slugger'

const SCHEME = /^[a-z][a-z\d+.-]*:/i
const SAFE_SCHEME = /^(?:https?|mailto):/i

export const REPOSITORY_MARKDOWN_HEADING_PREFIX = 'markdown-'

export function isRepositoryMarkdownPath(path: string) {
  const fileName = path.replace(/^\/+/, '').split('/').at(-1) ?? ''
  return /\.md$/i.test(fileName)
}

export function safeMarkdownUrl(url: string) {
  if (url.startsWith('#')) return markdownFragment(url.slice(1))
  if (SAFE_SCHEME.test(url)) return url
  return ''
}

export function resolveRepositoryMarkdownUrl(
  url: string,
  context: { markdownPath: string; owner: string; repo: string },
) {
  const safeUrl = safeMarkdownUrl(url)
  if (safeUrl) return safeUrl
  if (!url || SCHEME.test(url) || url.startsWith('//') || url.includes('?')) {
    return ''
  }

  const hashIndex = url.indexOf('#')
  const relativePath = hashIndex === -1 ? url : url.slice(0, hashIndex)
  const fragment =
    hashIndex === -1 ? '' : markdownFragment(url.slice(hashIndex + 1))
  const parts = relativePath.startsWith('/')
    ? []
    : context.markdownPath.replace(/^\/+/, '').split('/').slice(0, -1)

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
  const repositoryPath = `/${encodeURIComponent(context.owner)}/${encodeURIComponent(context.repo)}`
  return `${repositoryPath}?file=${encodeURIComponent(parts.join('/'))}${fragment}`
}

function markdownFragment(value: string) {
  if (/^user-content-fn(?:ref)?-/i.test(value)) return `#${value}`
  try {
    return `#${REPOSITORY_MARKDOWN_HEADING_PREFIX}${slug(decodeURIComponent(value))}`
  } catch {
    return ''
  }
}
