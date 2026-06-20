import { loadRepoForRequest, parseRepoParams } from '@/api/repos'
import {
  RepoDetailError,
  RepoDetailPage,
} from '@/features/repo-detail/repo-detail-page'
import { Outlet, createFileRoute, useChildMatches } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const loadRepo = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(({ data }) => loadRepoForRequest(data))

export const Route = createFileRoute('/repos/$owner/$repo')({
  loader: ({ params }) => loadRepo({ data: params }),
  errorComponent: RepoDetailError,
  component: RepoDetailRoute,
})

function RepoDetailRoute() {
  const childMatches = useChildMatches()
  if (childMatches.length > 0) {
    return <Outlet />
  }

  return <RepoDetailPage detail={Route.useLoaderData()} />
}
