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
import {
  requestChangeSelection,
  requestRevisionPin,
} from '@/features/requests/request-changes-model'
import { createRequestRevisionRedirectHandoff } from '@/features/requests/request-revision-navigation'
import { requestParamsForRoute } from '@/features/requests/request-route-data'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import { createFileRoute, getRouteApi, redirect } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

type LoadRequestRevisionsInput = ReturnType<typeof requestParamsForRoute> & {
  commit_oid?: string
  revision_id?: string
}

const requestRoute = getRouteApi('/$owner/$repo/requests/$requestId')
const revisionRedirectHandoff = typeof window === 'undefined'
  ? null
  : createRequestRevisionRedirectHandoff()

const loadRevisions = createServerFn({ method: 'GET' })
  .validator((data: LoadRequestRevisionsInput) => data)
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
    path: search.path,
    revision: search.revision,
  }),
  loader: async ({ deps, params }) => {
    const input = {
      ...requestParamsForRoute(params),
      commit_oid: deps.commit,
      revision_id: deps.revision,
    }
    const revisions = revisionRedirectHandoff?.take(input)
      ?? await loadRevisions({ data: input })
    if (!revisions) return null
    const selection = requestChangeSelection(
      revisions.revisions,
      revisions.review_revision_id,
      deps,
    )
    const pin = requestRevisionPin(
      selection.revision,
      selection.commit,
      deps.revision,
    )
    if (pin) {
      revisionRedirectHandoff?.stage({
        ...requestParamsForRoute(params),
        commit_oid: pin.commit,
        revision_id: pin.revision,
      }, revisions)
      throw redirect({
        params,
        replace: true,
        resetScroll: false,
        search: { ...pin, path: deps.path },
        to: '/$owner/$repo/requests/$requestId/changes',
      })
    }
    return revisions
  },
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
