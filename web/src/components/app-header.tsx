import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import { GitBranch } from 'lucide-react'
import type { ReactNode } from 'react'

type RenderSlot = ReactNode | (() => ReactNode)

type AppHeaderProps = {
  subtitle?: string
  subtitleClassName?: string
  contentClassName?: string
  homeLink?: boolean
  breadcrumb?: RenderSlot
  action?: RenderSlot
}

const APP_MARK = (
  <div className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-brand text-brand-foreground shadow-[var(--shadow-card)]">
    <GitBranch className="size-4.5" />
  </div>
)

export function AppHeader({
  action,
  breadcrumb,
  contentClassName,
  homeLink = true,
  subtitle,
  subtitleClassName,
}: AppHeaderProps) {
  const renderedAction = renderSlot(action)
  const renderedBreadcrumb = renderSlot(breadcrumb)
  const brand = renderedBreadcrumb ? (
    <div className="flex min-w-0 items-center gap-2.5">
      {homeLink ? (
        <Link className="flex shrink-0 items-center gap-2.5" to="/">
          {APP_MARK}
          <span className="hidden text-sm font-semibold tracking-tight sm:inline">
            Scope
          </span>
        </Link>
      ) : (
        <div className="flex shrink-0 items-center gap-2.5">
          {APP_MARK}
          <span className="hidden text-sm font-semibold tracking-tight sm:inline">
            Scope
          </span>
        </div>
      )}
      <span aria-hidden className="text-muted-foreground/50">
        /
      </span>
      <div className="min-w-0">{renderedBreadcrumb}</div>
    </div>
  ) : (
    <>
      {APP_MARK}
      <div className="min-w-0">
        <div className="truncate text-sm font-semibold leading-5 tracking-tight">
          Scope
        </div>
        {subtitle && (
          <div
            className={`truncate text-xs leading-4 text-muted-foreground ${subtitleClassName ?? ''}`}
          >
            {subtitle}
          </div>
        )}
      </div>
    </>
  )

  const brandWrapper =
    homeLink && !renderedBreadcrumb ? (
      <Link className="flex min-w-0 items-center gap-3" to="/">
        {brand}
      </Link>
    ) : (
      <div className="flex min-w-0 items-center gap-3">{brand}</div>
    )

  return (
    <header className="sticky top-0 z-30 border-b border-border bg-background/80 backdrop-blur-md supports-[backdrop-filter]:bg-background/65">
      <div
        className={cn(
          'mx-auto flex min-h-16 max-w-[1040px] items-center justify-between gap-3 px-4 py-3 sm:px-6',
          contentClassName,
        )}
      >
        {brandWrapper}
        {renderedAction}
      </div>
    </header>
  )
}

function renderSlot(slot: RenderSlot | undefined) {
  return typeof slot === 'function' ? slot() : slot
}
