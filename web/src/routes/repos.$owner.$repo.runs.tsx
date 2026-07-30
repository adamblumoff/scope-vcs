import { HttpError } from '@/api/client'
import {
  cancelRepoRunForRequest,
  loadRepoOperationsForRequest,
  loadRepoRunDetailForRequest,
  parseRunActionInput,
  retryRepoRunForRequest,
} from '@/api/runs'
import { parseRepoParams } from '@/api/repos'
import type { RepoParams, RunActionInput } from '@/api/types'
import {
  RepositoryRunsPage,
  RunsPageError,
  RunsPagePending,
} from '@/features/runs/repository-runs-page'
import { createFileRoute } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { useCallback } from 'react'

const loadRepoOperations = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(async ({ data }) => {
    try {
      return await loadRepoOperationsForRequest(data)
    } catch (error) {
      if (error instanceof HttpError && [401, 403, 404].includes(error.status)) {
        return null
      }
      throw error
    }
  })

const loadRepoRunDetail = createServerFn({ method: 'GET' })
  .validator(parseRunActionInput)
  .handler(({ data }) => loadRepoRunDetailForRequest(data))

const cancelRepoRun = createServerFn({ method: 'POST' })
  .validator(parseRunActionInput)
  .handler(({ data }) => cancelRepoRunForRequest(data))

const retryRepoRun = createServerFn({ method: 'POST' })
  .validator(parseRunActionInput)
  .handler(({ data }) => retryRepoRunForRequest(data))

export const Route = createFileRoute('/repos/$owner/$repo/runs')({
  loader: ({ params }) => loadRepoOperations({ data: params }),
  errorComponent: RunsPageError,
  pendingComponent: RunsPagePending,
  component: RepoRunsRoute,
})

function RepoRunsRoute() {
  const initialOperations = Route.useLoaderData()
  const params = Route.useParams()
  const loadOperations = useCallback(
    (input: RepoParams) => loadRepoOperations({ data: input }),
    [],
  )
  const loadDetail = useCallback(
    (input: RunActionInput) => loadRepoRunDetail({ data: input }),
    [],
  )
  const cancelRun = useCallback(
    async (input: RunActionInput) => {
      await cancelRepoRun({ data: input })
    },
    [],
  )
  const retryRun = useCallback(
    async (input: RunActionInput) => {
      await retryRepoRun({ data: input })
    },
    [],
  )

  return (
    <RepositoryRunsPage
      cancelRun={cancelRun}
      initialOperations={initialOperations}
      key={`${params.owner}/${params.repo}/${initialOperations ? 'member' : 'denied'}`}
      loadDetail={loadDetail}
      loadOperations={loadOperations}
      params={params}
      retryRun={retryRun}
    />
  )
}
