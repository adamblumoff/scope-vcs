import { HttpError } from '@/api/client'
import {
  loadRepoLiveStateForRequest,
  parseRepoParams,
} from '@/api/repos'
import { RepoShell } from '@/components/repo-shell'
import { RepositoryRoutePending } from '@/components/repository-route-pending'
import { RepositoryHtmlPreviewProvider } from '@/components/repository-html-preview-store'
import { RouteErrorPage } from '@/components/route-error-page'
import { RouteNotFoundPage } from '@/components/route-not-found-page'
import { RepoLayoutProvider } from '@/features/repo-detail/repo-layout-context'
import { useRepoLiveRefresh } from '@/features/repo-detail/repo-live-refresh'
import {
  Outlet,
  createFileRoute,
  notFound,
  useRouter,
} from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { useCallback } from 'react'

const loadRepoLiveState = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(async ({ data }) => {
    try {
      return await loadRepoLiveStateForRequest(data)
    } catch (error) {
      if (error instanceof HttpError && error.status === 404) {
        throw notFound()
      }
      throw error
    }
  })

export const Route = createFileRoute('/$owner/$repo')({
  loader: ({ params }) => loadRepoLiveState({ data: params }),
  errorComponent: RepoLayoutError,
  notFoundComponent: RouteNotFoundPage,
  pendingComponent: RepositoryRoutePending,
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
      <RepositoryHtmlPreviewProvider>
        <RepoShell params={params}>
          <Outlet />
        </RepoShell>
      </RepositoryHtmlPreviewProvider>
    </RepoLayoutProvider>
  )
}
