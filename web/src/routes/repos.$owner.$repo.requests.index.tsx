import {
  loadRepoLiveStateForRequest,
  loadRequestsForRequest,
  parseRepoParams,
} from '@/api/repos'
import { RequestsPage } from '@/features/requests/requests-page'
import { createFileRoute } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const loadRequestsPage = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(async ({ data }) => {
    const [live, list] = await Promise.all([
      loadRepoLiveStateForRequest(data),
      loadRequestsForRequest(data),
    ])

    return {
      live,
      requests: list.requests,
    }
  })

export const Route = createFileRoute('/repos/$owner/$repo/requests/')({
  loader: ({ params }) => loadRequestsPage({ data: params }),
  component: RequestsRoute,
})

function RequestsRoute() {
  const params = Route.useParams()
  const { live, requests } = Route.useLoaderData()

  return <RequestsPage live={live} params={params} requests={requests} />
}
