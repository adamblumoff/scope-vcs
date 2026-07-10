import type { RepoParams } from '@/api/types'
import { AppHeader } from '@/components/app-header'
import { AppShell } from '@/components/app-shell'
import { RepoBreadcrumb } from '@/components/repo-breadcrumb'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import { cn } from '@/lib/utils'
import { Link, useMatchRoute } from '@tanstack/react-router'
import type { ReactNode } from 'react'

export type RepoSection = 'code' | 'history' | 'requests' | 'settings'

const repoSections: Array<{
  key: RepoSection
  label: string
  to:
    | '/repos/$owner/$repo'
    | '/repos/$owner/$repo/history'
    | '/repos/$owner/$repo/requests'
    | '/repos/$owner/$repo/settings'
}> = [
  { key: 'code', label: 'Code', to: '/repos/$owner/$repo' },
  { key: 'requests', label: 'Requests', to: '/repos/$owner/$repo/requests' },
  { key: 'history', label: 'History', to: '/repos/$owner/$repo/history' },
  { key: 'settings', label: 'Settings', to: '/repos/$owner/$repo/settings' },
]

export function RepoShell({
  children,
  contentClassName,
  params,
}: {
  children: ReactNode
  contentClassName?: string
  params: RepoParams
}) {
  const { repo } = useRepoLayout()
  const matchRoute = useMatchRoute()
  const active = activeRepoSection(matchRoute)
  const canManage = repo.access.actor !== 'Public'
  return (
    <AppShell
      header={() => (
        <AppHeader
          breadcrumb={() => <RepoBreadcrumb params={params} />}
          contentClassName={contentClassName}
        />
      )}
      subheader={() => (
        <RepoNavigation
          active={active}
          canManage={canManage}
          contentClassName={contentClassName}
          params={params}
        />
      )}
    >
      {children}
    </AppShell>
  )
}

function activeRepoSection(matchRoute: ReturnType<typeof useMatchRoute>): RepoSection {
  if (matchRoute({ fuzzy: true, to: '/repos/$owner/$repo/settings' })) return 'settings'
  if (matchRoute({ fuzzy: true, to: '/repos/$owner/$repo/history' })) return 'history'
  if (matchRoute({ fuzzy: true, to: '/repos/$owner/$repo/requests' })) return 'requests'
  return 'code'
}

function RepoNavigation({
  active,
  canManage,
  contentClassName,
  params,
}: {
  active: RepoSection
  canManage: boolean
  contentClassName?: string
  params: RepoParams
}) {
  return (
    <div className="border-b border-border bg-background/80">
      <nav
        aria-label="Repository"
        className={cn(
          'mx-auto flex max-w-[1040px] gap-1 overflow-x-auto px-4 sm:px-6',
          contentClassName,
        )}
      >
        {repoSections.map((section) => {
          if (section.key === 'settings' && !canManage) {
            return null
          }
          const selected = active === section.key
          return (
            <Link
              aria-current={selected ? 'page' : undefined}
              className={cn(
                'flex h-11 shrink-0 items-center border-b-2 px-3 text-sm font-medium',
                selected
                  ? 'border-brand text-foreground'
                  : 'border-transparent text-muted-foreground hover:text-foreground',
              )}
              key={section.key}
              params={params}
              to={section.to}
            >
              {section.label}
            </Link>
          )
        })}
      </nav>
    </div>
  )
}
