import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import type { ReactNode } from 'react'
import { ScopeMark } from '@/components/scope-logo'
import { ThemeToggle } from '@/components/theme-toggle'
import { PageRail } from '@/components/page-header'

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
      <PageRail className="flex min-h-14 flex-wrap items-center gap-x-3 md:flex-nowrap md:gap-x-6">
        <Link
          aria-label="Scope home"
          className="flex shrink-0 items-center rounded-md text-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
          to="/"
        >
          <ScopeMark className="size-6" />
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

        {/*
          A single Primary nav for both breakpoints. Below `md` the row wraps
          and this takes its own full-bleed line; from `md` up it sits inline.
          Two navs would duplicate the landmark.
        */}
        {items.length > 0 && (
          <nav
            aria-label="Primary"
            className="scrollbar-none order-last -mx-5 flex h-11 w-[calc(100%+2.5rem)] items-center gap-5 overflow-x-auto border-t border-border px-5 text-[13px] font-medium md:order-none md:mx-0 md:ml-auto md:h-auto md:w-auto md:gap-1 md:overflow-visible md:border-t-0 md:px-0"
          >
            {items.map((item) => (
              <div
                className={cn(
                  'relative flex h-full shrink-0 items-center transition-colors after:absolute after:inset-x-0 after:bottom-0 after:h-0.5 after:rounded-full md:h-8 md:rounded-md md:after:hidden',
                  item.active
                    ? 'text-foreground after:bg-foreground md:bg-accent'
                    : 'text-muted-foreground after:bg-transparent hover:text-foreground md:hover:bg-accent/60',
                )}
                key={item.label}
              >
                {item.node}
              </div>
            ))}
          </nav>
        )}

        <div className="ml-auto flex shrink-0 items-center gap-1.5 md:ml-3">
          <ThemeToggle />
          {children}
        </div>
      </PageRail>
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
    <div className="flex min-w-0 flex-1 items-center gap-2 text-sm">
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
        <div className="flex min-w-0 items-center gap-2">
          {facts.map((fact) => (
            <span
              className="flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground"
              key={fact.id}
            >
              {fact.semantic && (
                <span
                  aria-hidden
                  className="size-2 rounded-full"
                  style={{ background: `var(--${fact.semantic})` }}
                />
              )}
              <span className="truncate">{fact.label}</span>
            </span>
          ))}
        </div>
      )}
    </div>
  )
}
