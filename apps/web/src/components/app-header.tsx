import { Button } from '@/components/ui/button'
import { Link } from '@tanstack/react-router'
import { ArrowLeft, GitBranch } from 'lucide-react'
import type { ReactNode } from 'react'

type AppHeaderProps = {
  subtitle: string
  subtitleClassName?: string
  homeLink?: boolean
  action?: ReactNode
}

export function AppHeader({
  action,
  homeLink = true,
  subtitle,
  subtitleClassName,
}: AppHeaderProps) {
  const brand = (
    <>
      <div className="flex size-8 shrink-0 items-center justify-center rounded-md border border-border">
        <GitBranch className="size-4" />
      </div>
      <div className="min-w-0">
        <div className="truncate text-sm font-semibold leading-5">Scope</div>
        <div
          className={`truncate text-xs leading-4 text-muted-foreground ${subtitleClassName ?? ''}`}
        >
          {subtitle}
        </div>
      </div>
    </>
  )

  return (
    <header className="border-b border-border bg-background">
      <div className="mx-auto flex min-h-16 max-w-[980px] items-center justify-between gap-3 px-4 py-3 sm:px-6">
        {homeLink ? (
          <Link className="flex min-w-0 items-center gap-3" to="/">
            {brand}
          </Link>
        ) : (
          <div className="flex min-w-0 items-center gap-3">{brand}</div>
        )}
        {action ?? (
          <Button asChild size="sm" variant="secondary">
            <Link to="/">
              <ArrowLeft className="size-3.5" />
              <span>Repos</span>
            </Link>
          </Button>
        )}
      </div>
    </header>
  )
}
