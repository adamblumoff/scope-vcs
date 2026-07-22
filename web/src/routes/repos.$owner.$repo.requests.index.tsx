import { loadRequestsForRequest } from '@/api/repos'
import { parseLoadRequestsInput } from '@/api/requests'
import { RequestsPage } from '@/features/requests/requests-page'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import { createFileRoute } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { useCallback } from 'react'

const loadRequestsPage = createServerFn({ method: 'GET' })
  .validator(parseLoadRequestsInput)
  .handler(async ({ data }) => loadRequestsForRequest(data))

export const Route = createFileRoute('/repos/$owner/$repo/requests/')({
  loader: async ({ params }) => ({
    initialPage: await loadRequestsPage({ data: params }),
    refreshKey: crypto.randomUUID(),
  }),
  component: RequestsRoute,
})

function RequestsRoute() {
  const params = Route.useParams()
  const { owner, repo } = params
  const live = useRepoLayout()
  const { initialPage, refreshKey } = Route.useLoaderData()
  const loadNextPage = useCallback(
    (cursor: string) => loadRequestsPage({ data: { owner, repo, cursor } }),
    [owner, repo],
  )

  return (
    <RequestsPage
      initialPage={initialPage}
      key={`${owner}/${repo}:${refreshKey}`}
      live={live}
      loadNextPage={loadNextPage}
      params={params}
    />
  )
}
