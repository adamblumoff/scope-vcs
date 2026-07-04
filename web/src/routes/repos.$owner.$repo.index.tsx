import {
  loadRepoForRequest,
  parseRepoParams,
} from '@/api/repos'
import { RepoDetailPage } from '@/features/repo-detail/repo-detail-page'
import { createFileRoute } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const loadRepo = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(({ data }) => loadRepoForRequest(data))

export const Route = createFileRoute('/repos/$owner/$repo/')({
  loader: ({ params }) => loadRepo({ data: params }),
  component: RepoIndexRoute,
})

function RepoIndexRoute() {
  const detail = Route.useLoaderData()
  const params = Route.useParams()

  return <RepoDetailPage detail={detail} params={params} />
}
