import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import { GitBranch } from 'lucide-react'
import type { ReactNode } from 'react'

type AppHeaderProps = {
  subtitle: string
  subtitleClassName?: string
  contentClassName?: string
  homeLink?: boolean
  action?: ReactNode
}

export function AppHeader({
  action,
  contentClassName,
  homeLink = true,
  subtitle,
  subtitleClassName,
}: AppHeaderProps) {
  const brand = (
    <>
      <div className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-brand text-brand-foreground shadow-[var(--shadow-card)]">
        <GitBranch className="size-4.5" />
      </div>
      <div className="min-w-0">
        <div className="truncate text-sm font-semibold leading-5 tracking-tight">
          Scope
        </div>
        <div
          className={`truncate text-xs leading-4 text-muted-foreground ${subtitleClassName ?? ''}`}
        >
          {subtitle}
        </div>
      </div>
    </>
  )

  return (
    <header className="sticky top-0 z-30 border-b border-border bg-background/80 backdrop-blur-md supports-[backdrop-filter]:bg-background/65">
      <div
        className={cn(
          'mx-auto flex min-h-16 max-w-[1040px] items-center justify-between gap-3 px-4 py-3 sm:px-6',
          contentClassName,
        )}
      >
        {homeLink ? (
          <Link className="flex min-w-0 items-center gap-3" to="/">
            {brand}
          </Link>
        ) : (
          <div className="flex min-w-0 items-center gap-3">{brand}</div>
        )}
        {action}
      </div>
    </header>
  )
}
