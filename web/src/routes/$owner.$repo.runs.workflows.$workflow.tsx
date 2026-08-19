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

export const Route = createFileRoute('/$owner/$repo/runs/workflows/$workflow')({
  loader: ({ params }) => loadRepoRunPage({ data: params }),
  errorComponent: RunsPageError,
  pendingComponent: RunsPagePending,
  component: RepoWorkflowRunsRoute,
})

function RepoWorkflowRunsRoute() {
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
      key={`${params.owner}/${params.repo}/${params.workflow}/${initialResources ? 'member' : 'denied'}`}
      loadDetail={loadDetail}
      loadHistory={loadHistory}
      params={params}
      workflow={params.workflow}
    />
  )
}
