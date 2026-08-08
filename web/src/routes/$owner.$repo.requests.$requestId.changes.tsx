import {
  type LoadRequestRevisionCommitInput,
  loadRequestRevisionCommitFileDiffForRequest,
  loadRequestRevisionCommitForRequest,
  loadRequestRevisionsForRequest,
} from '@/api/requests'
import {
  type LoadDiscussionsInput,
  loadRequestDiscussionsForRequest,
} from '@/features/requests/request-discussion-api'
import { RequestChangesView } from '@/features/requests/request-changes-view'
import { requestParamsForRoute } from '@/features/requests/request-route-data'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import { createFileRoute, getRouteApi } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const requestRoute = getRouteApi('/$owner/$repo/requests/$requestId')

const loadRevisions = createServerFn({ method: 'GET' })
  .validator((data: ReturnType<typeof requestParamsForRoute> & {
    commit_oid?: string
    revision_id?: string
  }) => data)
  .handler(async ({ data }) => {
    try {
      return await loadRequestRevisionsForRequest(data)
    } catch (error) {
      console.error('Loading request revisions failed', error)
      return null
    }
  })

const loadRevisionCommit = createServerFn({ method: 'GET' })
  .validator((data: LoadRequestRevisionCommitInput) => data)
  .handler(({ data }) => loadRequestRevisionCommitForRequest(data))

const loadRevisionDiff = createServerFn({ method: 'GET' })
  .validator((data: LoadRequestRevisionCommitInput & { path: string }) => data)
  .handler(({ data }) => loadRequestRevisionCommitFileDiffForRequest(data))

const loadDiscussions = createServerFn({ method: 'GET' })
  .validator((data: LoadDiscussionsInput) => data)
  .handler(({ data }) => loadRequestDiscussionsForRequest(data))

export const Route = createFileRoute(
  '/$owner/$repo/requests/$requestId/changes',
)({
  loaderDeps: ({ search }) => ({
    commit: search.commit,
    revision: search.revision,
  }),
  loader: ({ deps, params }) => loadRevisions({
    data: {
      ...requestParamsForRoute(params),
      commit_oid: deps.commit,
      revision_id: deps.revision,
    },
  }),
  component: RequestChangesRoute,
})

function RequestChangesRoute() {
  const page = requestRoute.useLoaderData()
  const revisions = Route.useLoaderData()
  const params = Route.useParams()
  const search = Route.useSearch()
  const navigate = Route.useNavigate()
  const live = useRepoLayout()
  const requestParams = requestParamsForRoute(params)

  if (!page.detail) return null

  return (
    <RequestChangesView
      audience={live.repo.access.can_read_private_files ? 'private' : 'public'}
      loadCommit={(data) => loadRevisionCommit({ data })}
      loadDiff={(data) => loadRevisionDiff({ data })}
      loadDiscussions={(data) => loadDiscussions({ data })}
      onSearchChange={(nextSearch) => {
        void navigate({
          params,
          replace: true,
          resetScroll: false,
          search: nextSearch,
          to: '/$owner/$repo/requests/$requestId/changes',
        })
      }}
      params={requestParams}
      repoId={live.repo.id}
      revisions={revisions}
      search={search}
    />
  )
}
