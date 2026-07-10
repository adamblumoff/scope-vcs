import {
  loadRequestsForRequest,
  parseRepoParams,
} from '@/api/repos'
import { RequestsPage } from '@/features/requests/requests-page'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import { createFileRoute } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const loadRequestsPage = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(async ({ data }) => (await loadRequestsForRequest(data)).requests)

export const Route = createFileRoute('/repos/$owner/$repo/requests/')({
  loader: ({ params }) => loadRequestsPage({ data: params }),
  component: RequestsRoute,
})

function RequestsRoute() {
  const params = Route.useParams()
  const live = useRepoLayout()
  const requests = Route.useLoaderData()

  return <RequestsPage live={live} params={params} requests={requests} />
}
