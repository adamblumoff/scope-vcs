import { HttpError } from '@/api/client'
import { loadRepoOperationsForRequest } from '@/api/runs'
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

export const Route = createFileRoute('/$owner/$repo/runs/')({
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

  return (
    <RepositoryRunsPage
      initialOperations={initialOperations}
      key={`${params.owner}/${params.repo}/${initialOperations ? 'member' : 'denied'}`}
      loadOperations={loadOperations}
      params={params}
    />
  )
}
