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

export function markdownClientNavigationHref(
  click: MarkdownLinkClick,
  currentHref: string,
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
