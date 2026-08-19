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
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import { UserButton } from '@clerk/tanstack-react-start'
import { Link, useLocation, useRouter } from '@tanstack/react-router'
import type { ReactNode } from 'react'

export function RepoShell({
  children,
  params,
}: {
  children: ReactNode
  params: RepoParams
}) {
  const { repo } = useRepoLayout()
  const router = useRouter()
  const pathname = useLocation({ select: (location) => location.pathname })
  const sections = repoSectionsForActor(repo.access.actor)
  const active = activeRepoSection((to) => {
    const target = router.buildLocation({ params, to }).pathname
    return pathname === target || pathname.startsWith(`${target}/`)
  })
  const items = sections.map<TopbarItem>(
    (section) => ({
      active: active === section.key,
      label: section.label,
      node: (
        <Link
          activeOptions={{ exact: section.key === 'code' }}
          aria-current={active === section.key ? 'page' : undefined}
          className="flex h-full items-center px-0 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring md:px-3"
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
        <ApplicationTopbar
          facts={[
            ...(repo.lifecycle_state === 'Ready'
              ? []
              : [{
                  id: 'lifecycle',
                  label: 'Awaiting first push',
                  semantic: 'warning' as const,
                }]),
            ...(repo.open_request_count > 0
              ? [{
                  id: 'requests',
                  label: `${repo.open_request_count} open`,
                }]
              : []),
          ]}
          items={items}
          repository={params}
        >
          <UserButton />
        </ApplicationTopbar>
      )}
    >
      {children}
    </AppShell>
  )
}
