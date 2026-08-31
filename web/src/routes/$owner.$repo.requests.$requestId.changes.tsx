import {
  type LoadRequestRevisionCommitInput,
  loadRequestRevisionCommitFileDiffForRequest,
  loadRequestRevisionsForRequest,
} from '@/api/requests'
import type { RequestParams, RequestRevisions } from '@/api/types'
import {
  type LoadDiscussionsInput,
  loadRequestDiscussionsForRequest,
} from '@/features/requests/request-discussion-api'
import { RequestChangesView } from '@/features/requests/request-changes-view'
import { loadCompleteDiscussionReferencePages } from '@/features/requests/request-changes-discussion-references'
import type {
  RequestChangesDiscussionReferences,
  RequestChangesSearch,
} from '@/features/requests/request-changes-workbench'
import { RequestChangesPending } from '@/features/requests/request-page-pending'
import {
  requestChangeSelection,
  requestRevisionCommitId,
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
    const revisions = await loadRequestRevisionsForRequest(data).catch((error: unknown) => {
      console.error('Loading request revisions failed', error)
      return null
    })
    const requestParams = {
      owner: data.owner,
      repo: data.repo,
      request_id: data.request_id,
    }
    const discussionPage = await loadRequestDiscussionsForRequest({
      ...requestParams,
      limit: 100,
    }).catch((error: unknown) => {
      console.error('Loading request discussion references failed', error)
      return null
    })
    const discussionReferences = await initialDiscussionReferences(
      requestParams,
      revisions,
      discussionPage,
    )
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
  loaderDeps: ({ search }) => requestChangesSelectionSearch(search),
  loader: async ({ deps: selectionSearch, params }) => {
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

async function initialDiscussionReferences(
  params: RequestParams,
  revisions: RequestRevisions | null,
  requestPage: Awaited<ReturnType<typeof loadRequestDiscussionsForRequest>> | null,
): Promise<RequestChangesDiscussionReferences> {
  if (requestPage && !requestPage.next_cursor) {
    return { all: requestPage, byCommit: {} }
  }
  if (!revisions) return { all: null, byCommit: {} }
  const queries = revisions.revisions.flatMap((revision) =>
    revision.commits.map((commit) => {
      const key = requestRevisionCommitId(revision.id, commit.oid)
      return {
        commit_oid: commit.oid,
        include_revision_anchor: commit.oid === revision.commits.at(-1)?.oid,
        key,
        revision_id: revision.id,
      }
    }),
  )
  const byCommit = await loadCompleteDiscussionReferencePages(
    queries.map((query) => ({
      input: {
        ...params,
        commit_oid: query.commit_oid,
        include_revision_anchor: query.include_revision_anchor,
        limit: 100,
        revision_id: query.revision_id,
      },
      key: query.key,
    })),
    loadRequestDiscussionsForRequest,
    (error) => {
      console.error('Loading request commit discussion references failed', error)
    },
  )
  return { all: null, byCommit }
}
