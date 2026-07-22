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
  loader: ({ params }) => loadRequestsPage({ data: params }),
  component: RequestsRoute,
})

function RequestsRoute() {
  const params = Route.useParams()
  const { owner, repo } = params
  const live = useRepoLayout()
  const initialPage = Route.useLoaderData()
  const loadNextPage = useCallback(
    (cursor: string) => loadRequestsPage({ data: { owner, repo, cursor } }),
    [owner, repo],
  )

  return (
    <RequestsPage
      initialPage={initialPage}
      key={`${params.owner}/${params.repo}`}
      live={live}
      loadNextPage={loadNextPage}
      params={params}
    />
  )
}
