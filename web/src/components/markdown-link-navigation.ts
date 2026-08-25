export type MarkdownLinkClick = {
  altKey: boolean
  button: number
  ctrlKey: boolean
  defaultPrevented: boolean
  download?: boolean | string
  href: string
  metaKey: boolean
  shiftKey: boolean
  target?: string
}

const NATIVE_DOCUMENT_PATHS = new Set([
  '/favicon.png',
  '/favicon.svg',
  '/robots.txt',
  '/sitemap.xml',
])
const NATIVE_DOCUMENT_PREFIXES = [
  '/.well-known/',
  '/_serverFn/',
  '/api/',
  '/assets/',
  '/brand/',
  '/v1/',
]

export function markdownClientNavigationHref(
  click: MarkdownLinkClick,
  currentHref: string,
  isClientRoute: (pathname: string) => boolean,
) {
  if (
    click.defaultPrevented ||
    click.button !== 0 ||
    click.altKey ||
    click.ctrlKey ||
    click.metaKey ||
    click.shiftKey ||
    click.target !== undefined ||
    (click.download !== undefined && click.download !== false) ||
    click.href.trimStart().startsWith('#')
  ) {
    return null
  }

  try {
    const current = new URL(currentHref)
    const destination = new URL(click.href, current)
    if (destination.origin !== current.origin) return null
    if (destination.protocol !== 'http:' && destination.protocol !== 'https:') {
      return null
    }
    if (
      NATIVE_DOCUMENT_PATHS.has(destination.pathname) ||
      NATIVE_DOCUMENT_PREFIXES.some((prefix) =>
        destination.pathname.startsWith(prefix)
      ) ||
      !isClientRoute(destination.pathname)
    ) {
      return null
    }
    if (
      destination.hash &&
      destination.pathname === current.pathname &&
      destination.search === current.search
    ) {
      return null
    }
    return `${destination.pathname}${destination.search}${destination.hash}`
  } catch {
    return null
  }
}
