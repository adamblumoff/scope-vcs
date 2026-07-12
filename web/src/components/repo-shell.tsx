import type { RepoParams } from '@/api/types'
import {
  ApplicationTopbar,
  type TopbarItem,
} from '@/components/application-topbar'
import { AppShell } from '@/components/app-shell'
import { RepositoryContextStrip } from '@/components/repository-context-strip'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import { cn } from '@/lib/utils'
import { UserButton } from '@clerk/tanstack-react-start'
import { Link, useMatchRoute } from '@tanstack/react-router'
import type { ReactNode } from 'react'

export type RepoSection = 'code' | 'history' | 'requests' | 'settings'

const repoSections = [
  { key: 'code', label: 'Code', to: '/repos/$owner/$repo' },
  { key: 'requests', label: 'Requests', to: '/repos/$owner/$repo/requests' },
  { key: 'history', label: 'History', to: '/repos/$owner/$repo/history' },
  { key: 'settings', label: 'Settings', to: '/repos/$owner/$repo/settings' },
] as const

export function RepoShell({
  actions,
  children,
  className,
  contentClassName,
  params,
}: {
  actions?: ReactNode
  children: ReactNode
  className?: string
  contentClassName?: string
  params: RepoParams
}) {
  const { repo } = useRepoLayout()
  const active = activeRepoSection(useMatchRoute())
  const items = repoSections.flatMap<TopbarItem>((section) => {
    if (section.key === 'settings' && repo.access.actor === 'Public') return []
    return [{
      active: active === section.key,
      label: section.label,
      node: (
        <Link
          aria-current={active === section.key ? 'page' : undefined}
          className="flex h-full items-center focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
          params={params}
          to={section.to}
        >
          {section.label}
        </Link>
      ),
    }]
  })

  return (
    <AppShell
      className={className}
      header={() => (
        <ApplicationTopbar items={items} repository={params}>
          {actions}
          <UserButton />
        </ApplicationTopbar>
      )}
      subheader={() => (
        <RepositoryContextStrip
          facts={[
            { id: 'lifecycle', label: repo.lifecycle_state, semantic: repo.lifecycle_state === 'Published' ? 'success' : 'warning' },
            { id: 'visibility', label: repo.default_visibility },
            ...(repo.access.actor === 'Public'
              ? []
              : [{ id: 'actor', label: repo.access.actor }]),
            ...(repo.open_request_count > 0
              ? [{ id: 'requests', label: `${repo.open_request_count} open request${repo.open_request_count === 1 ? '' : 's'}` }]
              : []),
          ]}
        />
      )}
    >
      <div className={cn('mx-auto w-full max-w-[1440px]', contentClassName)}>
        {children}
      </div>
    </AppShell>
  )
}

function activeRepoSection(
  matchRoute: ReturnType<typeof useMatchRoute>,
): RepoSection {
  if (matchRoute({ fuzzy: true, to: '/repos/$owner/$repo/settings' })) return 'settings'
  if (matchRoute({ fuzzy: true, to: '/repos/$owner/$repo/history' })) return 'history'
  if (matchRoute({ fuzzy: true, to: '/repos/$owner/$repo/requests' })) return 'requests'
  return 'code'
}
