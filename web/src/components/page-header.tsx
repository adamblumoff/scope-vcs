import { cn } from '@/lib/utils'
import type { ReactNode } from 'react'

/**
 * Padded reading column for list and prose routes. Shares the app rail width
 * with the topbar so page content lines up with the logo and nav.
 */
export function PageContent({
  children,
  className,
}: {
  children: ReactNode
  className?: string
}) {
  return (
    <section
      className={cn(
        'mx-auto w-full max-w-[1280px] px-5 py-8 sm:px-6 lg:px-8 lg:py-10',
        className,
      )}
    >
      {children}
    </section>
  )
}

/**
 * Same rail as `PageContent` but unpadded, for split-pane workbenches whose
 * panels manage their own edges (code, history, runs, diffs).
 */
export function WorkbenchPane({
  children,
  className,
}: {
  children: ReactNode
  className?: string
}) {
  return (
    <div className={cn('mx-auto w-full max-w-[1280px]', className)}>
      {children}
    </div>
  )
}

/**
 * The single page-title treatment. Repo section routes deliberately do not use
 * this — their nav tab already names the view — and reach for `WorkbenchBar`.
 */
export function PageHeader({
  actions,
  badges,
  children,
  description,
  title,
}: {
  actions?: ReactNode
  badges?: ReactNode
  children?: ReactNode
  description?: ReactNode
  title: ReactNode
  }) {
  return (
    <header className="flex flex-col gap-5 sm:flex-row sm:items-start sm:justify-between">
      <div className="min-w-0">
        <h1 className="break-words text-[26px] font-semibold leading-[1.15] tracking-[-0.02em] sm:text-[32px]">
          {title}
        </h1>
        {description && (
          <p className="mt-2 max-w-[62ch] text-[15px] leading-6 text-muted-foreground">
            {description}
          </p>
        )}
        {badges && (
          <div className="mt-3 flex flex-wrap items-center gap-1.5">
            {badges}
          </div>
        )}
        {children}
      </div>
      {actions && (
        <div className="flex shrink-0 flex-wrap items-center gap-2 sm:justify-end">
          {actions}
        </div>
      )}
    </header>
  )
}

/**
 * Thin utility bar for repo section routes: a plain-language summary on the
 * left, controls on the right. No title, no eyebrow — the nav supplies those.
 */
export function WorkbenchBar({
  actions,
  className,
  summary,
}: {
  actions?: ReactNode
  className?: string
  summary?: ReactNode
}) {
  if (!actions && !summary) return null

  return (
    <div
      className={cn(
        'flex min-h-14 flex-wrap items-center justify-between gap-x-4 gap-y-2 px-5 py-3 sm:px-6 lg:px-8',
        className,
      )}
    >
      <div className="min-w-0 text-sm text-muted-foreground">{summary}</div>
      {actions && (
        <div className="flex shrink-0 flex-wrap items-center gap-2">
          {actions}
        </div>
      )}
    </div>
  )
}
