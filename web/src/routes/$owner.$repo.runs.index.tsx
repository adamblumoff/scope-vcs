import type { RepoRunHistoryInput, RunActionInput } from '@/api/types'
import {
  RepositoryRunsPage,
} from '@/features/runs/repository-runs-page'
import { RunsPagePending } from '@/features/runs/runs-page-pending'
import { RunsPageError } from '@/features/runs/runs-page-error'
import {
  loadRepoRunHistory,
  loadRepoRunDetail,
  loadRepoRunPage,
} from '@/routes/-run-history-actions'
import { createFileRoute } from '@tanstack/react-router'
import { useCallback } from 'react'

export const Route = createFileRoute('/$owner/$repo/runs/')({
  loader: ({ params }) => loadRepoRunPage({ data: params }),
  errorComponent: RunsPageError,
  pendingComponent: RunsPagePending,
  component: RepoRunsRoute,
})

function RepoRunsRoute() {
  const initialResources = Route.useLoaderData()
  const params = Route.useParams()
  const loadHistory = useCallback(
    (input: RepoRunHistoryInput) => loadRepoRunHistory({ data: input }),
    [],
  )
  const loadDetail = useCallback(
    (input: RunActionInput) => loadRepoRunDetail({ data: input }),
    [],
  )

  return (
    <RepositoryRunsPage
      initialResources={initialResources}
      key={`${params.owner}/${params.repo}/all/${initialResources ? 'member' : 'denied'}`}
      loadDetail={loadDetail}
      loadHistory={loadHistory}
      params={params}
    />
  )
}
