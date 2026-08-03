import type { RepoParams } from '@/api/types'
import {
  ApplicationTopbar,
  type TopbarItem,
} from '@/components/application-topbar'
import { AppShell } from '@/components/app-shell'
import {
  activeRepoSection,
  repoSectionsForActor,
} from '@/components/repo-section-model'
import { RepositoryContextStrip } from '@/components/repository-context-strip'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import { UserButton } from '@clerk/tanstack-react-start'
import { Link, useMatchRoute } from '@tanstack/react-router'
import type { ReactNode } from 'react'

export function RepoShell({
  children,
  params,
}: {
  children: ReactNode
  params: RepoParams
}) {
  const { repo } = useRepoLayout()
  const matchRoute = useMatchRoute()
  const active = activeRepoSection((to) => Boolean(matchRoute({
    fuzzy: true,
    to,
  })))
  const items = repoSectionsForActor(repo.access.actor).map<TopbarItem>(
    (section) => ({
      active: active === section.key,
      label: section.label,
      node: (
        <Link
          activeOptions={{ exact: section.key === 'code' }}
          aria-current={active === section.key ? 'page' : undefined}
          className="flex h-full items-center px-0 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring sm:px-3"
          params={params}
          to={section.to}
        >
          {section.label}
        </Link>
      ),
    }),
  )

  return (
    <AppShell
      header={() => (
        <ApplicationTopbar items={items} repository={params}>
          <UserButton />
        </ApplicationTopbar>
      )}
      subheader={() => (
        <RepositoryContextStrip
          facts={[
            { id: 'lifecycle', label: repo.lifecycle_state, semantic: repo.lifecycle_state === 'Ready' ? 'success' : 'warning' },
            ...(repo.access.actor === 'Public'
              ? []
              : [{ id: 'actor', label: repo.access.actor }]),
            ...(repo.open_request_count > 0
              ? [{ id: 'requests', label: `${repo.open_request_count} open` }]
              : []),
          ]}
        />
      )}
    >
      <div className="mx-auto w-full max-w-[1440px]">
        {children}
      </div>
    </AppShell>
  )
}
