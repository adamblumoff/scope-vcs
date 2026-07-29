import {
  loadRepoLiveStateForRequest,
  parseRepoParams,
} from '@/api/repos'
import { RepoShell } from '@/components/repo-shell'
import { RouteErrorPage } from '@/components/route-error-page'
import { RepoLayoutProvider } from '@/features/repo-detail/repo-layout-context'
import { useRepoLiveRefresh } from '@/features/repo-detail/repo-live-refresh'
import { Outlet, createFileRoute, useRouter } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { useCallback } from 'react'

const loadRepoLiveState = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(({ data }) => loadRepoLiveStateForRequest(data))

export const Route = createFileRoute('/repos/$owner/$repo')({
  loader: ({ params }) => loadRepoLiveState({ data: params }),
  errorComponent: RepoLayoutError,
  component: RepoLayoutRoute,
})

function RepoLayoutError({ error }: { error: unknown }) {
  return (
    <RouteErrorPage
      error={error}
      fallbackMessage="Unexpected repository error"
      title="Repository unavailable"
    />
  )
}

function RepoLayoutRoute() {
  const live = Route.useLoaderData()
  const params = Route.useParams()
  const router = useRouter()
  const invalidate = useCallback(() => router.invalidate(), [router])
  const subscribe = useRepoLiveRefresh(live, invalidate)
  return (
    <RepoLayoutProvider live={live} subscribe={subscribe}>
      <RepoShell params={params}>
        <Outlet />
      </RepoShell>
    </RepoLayoutProvider>
  )
}
