import { useRouter } from '@tanstack/react-router'
import type { ComponentProps, MouseEvent } from 'react'
import { markdownClientNavigationHref } from './markdown-link-navigation'

export function MarkdownLink({
  children,
  download,
  href,
  onClick,
  target,
  ...props
}: ComponentProps<'a'>) {
  const router = useRouter()

  if (!href) return <span className={props.className}>{children}</span>

  const handleClick = (event: MouseEvent<HTMLAnchorElement>) => {
    onClick?.(event)
    const destination = markdownClientNavigationHref(
      {
        altKey: event.altKey,
        button: event.button,
        ctrlKey: event.ctrlKey,
        defaultPrevented: event.defaultPrevented,
        download,
        href,
        metaKey: event.metaKey,
        shiftKey: event.shiftKey,
        target,
      },
      window.location.href,
    )
    if (!destination) return

    event.preventDefault()
    void router.navigate({ href: destination })
  }

  return (
    <a
      {...props}
      download={download}
      href={href}
      onClick={handleClick}
      rel={props.rel ?? 'noreferrer'}
      target={target}
    >
      {children}
    </a>
  )
}
