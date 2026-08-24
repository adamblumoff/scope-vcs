import {
  type LoadRequestRevisionCommitInput,
  loadRequestRevisionCommitFileDiffForRequest,
  loadRequestRevisionsForRequest,
} from '@/api/requests'
import {
  type LoadDiscussionsInput,
  loadRequestDiscussionsForRequest,
} from '@/features/requests/request-discussion-api'
import { RequestChangesView } from '@/features/requests/request-changes-view'
import type { RequestChangesSearch } from '@/features/requests/request-changes-workbench'
import { RequestChangesPending } from '@/features/requests/request-page-pending'
import {
  requestChangeSelection,
  requestRevisionPin,
} from '@/features/requests/request-changes-model'
import { requestParamsForRoute } from '@/features/requests/request-route-data'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import { createFileRoute, getRouteApi } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { useEffect, useMemo } from 'react'

type LoadRequestRevisionsInput = ReturnType<typeof requestParamsForRoute> & {
  commit_oid?: string
  revision_id?: string
}

const requestRoute = getRouteApi('/$owner/$repo/requests/$requestId')

const loadChangesPage = createServerFn({ method: 'GET' })
  .validator((data: LoadRequestRevisionsInput) => data)
  .handler(async ({ data }) => {
    const [revisions, discussionReferences] = await Promise.all([
      loadRequestRevisionsForRequest(data).catch((error: unknown) => {
        console.error('Loading request revisions failed', error)
        return null
      }),
      loadRequestDiscussionsForRequest({ ...data, limit: 100 }).catch((error: unknown) => {
        console.error('Loading request discussion references failed', error)
        return null
      }),
    ])
    return { discussionReferences, revisions }
  })

const loadRevisionDiff = createServerFn({ method: 'GET' })
  .validator((data: LoadRequestRevisionCommitInput & { path: string }) => data)
  .handler(({ data }) => loadRequestRevisionCommitFileDiffForRequest(data))

const loadDiscussions = createServerFn({ method: 'GET' })
  .validator((data: LoadDiscussionsInput) => data)
  .handler(({ data }) => loadRequestDiscussionsForRequest(data))

const loadDiffForView = (data: LoadRequestRevisionCommitInput & { path: string }) =>
  loadRevisionDiff({ data })
const loadDiscussionsForView = (data: LoadDiscussionsInput) =>
  loadDiscussions({ data })

export const Route = createFileRoute(
  '/$owner/$repo/requests/$requestId/changes',
)({
  loaderDeps: () => ({}),
  loader: async ({ location, params }) => {
    const selectionSearch = requestChangesSelectionSearch(location.search)
    const input = {
      ...requestParamsForRoute(params),
      commit_oid: selectionSearch.commit,
      revision_id: selectionSearch.revision,
    }
    const page = await loadChangesPage({ data: input })
    const { revisions } = page
    if (!revisions) return { ...page, pin: null }
    const selection = requestChangeSelection(
      revisions.revisions,
      revisions.review_revision_id,
      selectionSearch,
    )
    const pin = requestRevisionPin(
      selection.revision,
      selection.commit,
      selectionSearch.revision,
    )
    return { ...page, pin }
  },
  pendingComponent: RequestChangesPending,
  component: RequestChangesRoute,
})

function RequestChangesRoute() {
  const page = requestRoute.useLoaderData()
  const changes = Route.useLoaderData()
  const params = Route.useParams()
  const search = Route.useSearch()
  const navigate = Route.useNavigate()
  const live = useRepoLayout()
  const { owner, repo, requestId } = params
  const requestParams = useMemo(
    () => requestParamsForRoute({ owner, repo, requestId }),
    [owner, repo, requestId],
  )
  useEffect(() => {
    if (!changes.pin || search.revision) return
    void navigate({
      params,
      replace: true,
      resetScroll: false,
      search: (current) => ({ ...current, ...changes.pin }),
      to: '/$owner/$repo/requests/$requestId/changes',
    })
  }, [changes.pin, navigate, params, search.revision])

  if (!page.detail) return null

  return (
    <RequestChangesView
      audience={live.repo.access.can_read_private_files ? 'private' : 'public'}
      initialDiscussionReferences={changes.discussionReferences}
      loadDiff={loadDiffForView}
      loadDiscussions={loadDiscussionsForView}
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
      revisions={changes.revisions}
      search={search}
    />
  )
}

function requestChangesSelectionSearch(search: unknown): RequestChangesSearch {
  if (!search || typeof search !== 'object') return {}
  const values = search as Record<string, unknown>
  return {
    commit: typeof values.commit === 'string' ? values.commit : undefined,
    revision: typeof values.revision === 'string' ? values.revision : undefined,
  }
}
