import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import { GitBranch } from 'lucide-react'
import type { ReactNode } from 'react'
import { SearchControl } from '@/components/ui/search-control'

export type TopbarItem = {
  active?: boolean
  label: string
  node: ReactNode
}

const EMPTY_ITEMS: TopbarItem[] = []

export function ApplicationTopbar({
  children,
  contextLabel,
  items = EMPTY_ITEMS,
  repository,
  searchPreview = false,
}: {
  children?: ReactNode
  contextLabel?: string
  items?: TopbarItem[]
  repository?: { owner: string; repo: string }
  searchPreview?: boolean
}) {
  return (
    <header className="sticky top-0 z-40 border-b border-border bg-card/95 backdrop-blur supports-[backdrop-filter]:bg-card/88">
      <div className="mx-auto grid min-h-16 max-w-[1440px] grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-x-3 px-4 sm:gap-x-6 sm:px-6 lg:px-8">
        <div className="col-start-1 flex min-w-0 items-center gap-3 sm:gap-5">
          <Link
            aria-label="Scope home"
            className="flex shrink-0 items-center gap-2.5 rounded-md text-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
            to="/"
          >
            <GitBranch className="size-[18px] text-[var(--platinum-bright)]" strokeWidth={1.8} />
            <span className="hidden text-[17px] font-semibold tracking-[-0.025em] sm:inline">
              Scope
            </span>
          </Link>
          {repository ? (
            <div className="min-w-0">
              <RepositoryIdentity owner={repository.owner} repo={repository.repo} />
            </div>
          ) : contextLabel ? (
            <span className="truncate text-sm text-muted-foreground">{contextLabel}</span>
          ) : null}
        </div>

        {items.length > 0 && (
          <nav
            aria-label="Primary"
            className="col-span-3 col-start-1 row-start-2 -mx-4 flex h-11 min-w-0 gap-6 overflow-x-auto border-t border-border px-4 sm:col-span-1 sm:col-start-2 sm:row-start-1 sm:mx-0 sm:h-16 sm:justify-center sm:border-t-0 sm:px-0"
          >
            {items.map((item) => (
              <div
                className={cn(
                  'relative flex h-full shrink-0 items-center text-sm font-medium text-muted-foreground transition-colors after:absolute after:inset-x-0 after:bottom-0 after:h-0.5 after:rounded-full after:bg-transparent',
                  item.active
                    ? 'text-foreground after:bg-[var(--platinum-bright)]'
                    : 'hover:text-foreground',
                )}
                key={item.label}
              >
                {item.node}
              </div>
            ))}
          </nav>
        )}

        <div className="col-start-3 flex shrink-0 items-center justify-end gap-2">
          {searchPreview ? <SearchControl /> : null}
          {children}
        </div>
      </div>
    </header>
  )
}

function RepositoryIdentity({
  owner,
  repo,
}: {
  owner: string
  repo: string
}) {
  return (
    <Link
      className="flex min-w-0 max-w-[calc(100vw-210px)] items-center gap-1 rounded-md border border-[var(--border-strong)] bg-secondary px-3 py-2 text-sm shadow-[var(--shadow-card)] transition-colors hover:bg-accent sm:max-w-[360px]"
      params={{ owner, repo }}
      to="/repos/$owner/$repo"
    >
      <span className="truncate text-muted-foreground">{owner}</span>
      <span aria-hidden className="text-muted-foreground/60">/</span>
      <span className="truncate font-medium text-foreground">{repo}</span>
    </Link>
  )
}
