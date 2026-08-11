import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import type { ReactNode } from 'react'
import { ScopeLogo, ScopeMark } from '@/components/scope-logo'
import { ThemeToggle } from '@/components/theme-toggle'

export type TopbarItem = {
  active?: boolean
  label: string
  node: ReactNode
}

/** Compact repository facts rendered beside the repo name. */
export type TopbarFact = {
  id: string
  label: ReactNode
  semantic?: 'danger' | 'info' | 'success' | 'warning'
}

const EMPTY_ITEMS: TopbarItem[] = []
const EMPTY_FACTS: TopbarFact[] = []

export function ApplicationTopbar({
  children,
  contextLabel,
  facts = EMPTY_FACTS,
  items = EMPTY_ITEMS,
  repository,
}: {
  children?: ReactNode
  contextLabel?: string
  facts?: TopbarFact[]
  items?: TopbarItem[]
  repository?: { owner: string; repo: string }
}) {
  return (
    <header className="sticky top-0 z-40 border-b border-border bg-card">
      <div className="mx-auto flex min-h-14 max-w-[1280px] items-center gap-x-3 px-5 sm:gap-x-6 sm:px-6 lg:px-8">
        <Link
          aria-label="Scope home"
          className="flex shrink-0 items-center rounded-md text-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
          to="/"
        >
          <ScopeMark className="size-6 sm:hidden" />
          <ScopeLogo className="hidden w-[86px] sm:block" />
        </Link>

        {repository ? (
          <RepositoryIdentity
            facts={facts}
            owner={repository.owner}
            repo={repository.repo}
          />
        ) : contextLabel ? (
          <span className="min-w-0 truncate text-sm text-muted-foreground">
            {contextLabel}
          </span>
        ) : null}

        {items.length > 0 && (
          <nav
            aria-label="Primary"
            className="ml-auto hidden items-center gap-1 sm:flex"
          >
            {items.map((item) => (
              <div
                className={cn(
                  'relative flex h-8 shrink-0 items-center rounded-md text-[13px] font-medium transition-colors',
                  item.active
                    ? 'bg-accent text-foreground'
                    : 'text-muted-foreground hover:bg-accent/60 hover:text-foreground',
                )}
                key={item.label}
              >
                {item.node}
              </div>
            ))}
          </nav>
        )}

        <div
          className={cn(
            'flex shrink-0 items-center gap-1.5',
            items.length > 0 ? 'ml-auto sm:ml-3' : 'ml-auto',
          )}
        >
          <ThemeToggle />
          {children}
        </div>
      </div>

      {items.length > 0 && (
        <nav
          aria-label="Primary"
          className="flex h-11 items-center gap-5 overflow-x-auto border-t border-border px-5 text-[13px] font-medium sm:hidden"
        >
          {items.map((item) => (
            <div
              className={cn(
                'relative flex h-full shrink-0 items-center after:absolute after:inset-x-0 after:bottom-0 after:h-0.5 after:rounded-full',
                item.active
                  ? 'text-foreground after:bg-foreground'
                  : 'text-muted-foreground after:bg-transparent',
              )}
              key={item.label}
            >
              {item.node}
            </div>
          ))}
        </nav>
      )}
    </header>
  )
}

function RepositoryIdentity({
  facts,
  owner,
  repo,
}: {
  facts: TopbarFact[]
  owner: string
  repo: string
}) {
  return (
    <div className="flex min-w-0 items-center gap-2 text-sm">
      <div className="flex min-w-0 items-baseline gap-1.5">
        <Link
          className="hidden max-w-[130px] truncate rounded-md text-[13px] text-muted-foreground transition-colors hover:text-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring sm:block"
          params={{ owner }}
          title={owner}
          to="/$owner"
        >
          {owner}
        </Link>
        <span aria-hidden className="hidden text-muted-foreground/50 sm:inline">
          /
        </span>
        <Link
          className="min-w-0 truncate rounded-md text-[14px] font-semibold tracking-[-0.01em] text-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
          params={{ owner, repo }}
          title={`${owner}/${repo}`}
          to="/$owner/$repo"
        >
          {repo}
        </Link>
      </div>
      {facts.length > 0 && (
        <div className="hidden shrink-0 items-center gap-2 md:flex">
          {facts.map((fact) => (
            <span
              className="flex items-center gap-1.5 text-xs text-muted-foreground"
              key={fact.id}
            >
              {fact.semantic && (
                <span
                  aria-hidden
                  className="size-2 rounded-full"
                  style={{ background: `var(--${fact.semantic})` }}
                />
              )}
              {fact.label}
            </span>
          ))}
        </div>
      )}
    </div>
  )
}
