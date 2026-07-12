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
    <header className="sticky top-0 z-40 border-b border-border bg-[linear-gradient(180deg,var(--topbar-start),var(--topbar-end))] shadow-[var(--topbar-shadow)] backdrop-blur-xl">
      <div className="mx-auto grid min-h-[68px] max-w-[1440px] grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-x-3 px-4 sm:gap-x-6 sm:px-6 lg:px-8">
        <div className="col-start-1 flex min-w-0 items-center gap-3 sm:gap-5">
          <Link
            aria-label="Scope home"
            className="group flex shrink-0 items-center gap-2.5 rounded-md text-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
            to="/"
          >
            <span className="flex size-7 items-center justify-center rounded-full border border-border bg-[var(--topbar-rail)] shadow-[inset_0_1px_rgba(255,255,255,0.06)] transition-transform duration-150 group-hover:-translate-y-px motion-reduce:transform-none">
              <GitBranch className="size-[15px] text-[var(--platinum-bright)]" strokeWidth={1.9} />
            </span>
            <span className="hidden text-[18px] font-semibold tracking-[-0.035em] sm:inline">
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
            className="col-span-3 col-start-1 row-start-2 -mx-4 flex h-11 min-w-0 gap-6 overflow-x-auto border-t border-border px-4 sm:col-span-1 sm:col-start-2 sm:row-start-1 sm:mx-0 sm:h-auto sm:w-fit sm:justify-self-center sm:gap-1 sm:overflow-visible sm:rounded-lg sm:border sm:border-border sm:bg-[var(--topbar-rail)] sm:p-1"
          >
            {items.map((item) => (
              <div
                className={cn(
                  'relative flex h-full shrink-0 items-center rounded-md text-[13px] font-medium text-muted-foreground transition-[color,background-color,box-shadow,transform] duration-150 after:absolute after:inset-x-0 after:bottom-0 after:h-0.5 after:rounded-full after:bg-transparent sm:h-8 sm:after:hidden motion-reduce:transition-none',
                  item.active
                    ? 'text-foreground after:bg-[var(--platinum-bright)] sm:bg-[linear-gradient(180deg,var(--topbar-active-start),var(--topbar-active-end))] sm:shadow-[var(--topbar-active-shadow)]'
                    : 'hover:text-foreground sm:hover:-translate-y-px sm:hover:bg-accent/60 motion-reduce:transform-none',
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
      className="group flex min-w-0 max-w-[calc(100vw-205px)] items-baseline gap-1.5 rounded-md py-2 text-sm focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring sm:max-w-[360px]"
      params={{ owner, repo }}
      title={`${owner}/${repo}`}
      to="/repos/$owner/$repo"
    >
      <span className="max-w-[38%] truncate text-[12px] font-medium text-muted-foreground transition-colors group-hover:text-foreground/80 sm:max-w-[130px] sm:text-[13px]">{owner}</span>
      <span aria-hidden className="text-muted-foreground/45">/</span>
      <span className="truncate text-[14px] font-semibold tracking-[-0.015em] text-foreground">{repo}</span>
    </Link>
  )
}
