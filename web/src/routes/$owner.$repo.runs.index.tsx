import { createApiClient, HttpError } from '@/api/client'
import {
  loadRepoRunHistoryForRequest,
  loadRepoRunnersForRequest,
} from '@/api/runs'
import { parseRepoParams } from '@/api/repos'
import type { RepoParams } from '@/api/types'
import {
  RepositoryRunsPage,
  RunsPageError,
  RunsPagePending,
} from '@/features/runs/repository-runs-page'
import { createFileRoute } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { useCallback } from 'react'

const loadRepoRunResources = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(async ({ data }) => {
    try {
      const api = createApiClient()
      const [history, runners] = await Promise.all([
        loadRepoRunHistoryForRequest(data, api),
        loadRepoRunnersForRequest(data, api),
      ])
      return { history, runners }
    } catch (error) {
      if (error instanceof HttpError && [401, 403, 404].includes(error.status)) {
        return null
      }
      throw error
    }
  })

export const Route = createFileRoute('/$owner/$repo/runs/')({
  loader: ({ params }) => loadRepoRunResources({ data: params }),
  errorComponent: RunsPageError,
  pendingComponent: RunsPagePending,
  component: RepoRunsRoute,
})

function RepoRunsRoute() {
  const initialResources = Route.useLoaderData()
  const params = Route.useParams()
  const loadResources = useCallback(
    (input: RepoParams) => loadRepoRunResources({ data: input }),
    [],
  )

  return (
    <RepositoryRunsPage
      initialResources={initialResources}
      key={`${params.owner}/${params.repo}/${initialResources ? 'member' : 'denied'}`}
      loadResources={loadResources}
      params={params}
    />
  )
}
