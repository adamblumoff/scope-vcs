import {
  parseLoadRequestRevisionsInput,
  parseLoadRequestRevisionDiffInput,
  parseLoadDiscussionsInput,
} from '@/api/request-inputs'
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
import { loadDiscussionReferencePage, selectedDiscussionReferenceQuery } from '@/features/requests/request-changes-discussion-references'
import type {
  RequestChangesDiscussionReferences,
  RequestChangesSearch,
} from '@/features/requests/request-changes-workbench'
import { RequestChangesPending } from '@/features/requests/request-page-pending'
import {
  requestChangeSelection,
  requestRevisionPin,
} from '@/features/requests/request-changes-model'
import { requestParamsForRoute } from '@/features/requests/request-route-data'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import { createFileRoute, getRouteApi } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { getRequest } from '@tanstack/react-start/server'
import { useEffect, useMemo } from 'react'

type LoadRequestRevisionsInput = ReturnType<typeof parseLoadRequestRevisionsInput>

type ChangesPage = Awaited<ReturnType<typeof loadChangesPage>>
type ChangesLoaderData = ChangesPage & {
  pin: RequestChangesSearch | null
}

type PinnedChangesReplay = {
  data: ChangesLoaderData
  key: string
}

const pinnedChangesReplay: { current: PinnedChangesReplay | null } = { current: null }

const requestRoute = getRouteApi('/$owner/$repo/requests/$requestId')

const loadChangesPage = createServerFn({ method: 'GET' })
  .validator(parseLoadRequestRevisionsInput)
  .handler(async ({ data }) => {
    const revisions = await loadRequestRevisionsForRequest(data).catch((error: unknown) => {
      console.error('Loading request revisions failed', error)
      return null
    })
    const query = revisions ? selectedDiscussionReferenceQuery(data, revisions) : null
    const page = query
      ? await loadDiscussionReferencePage(query.input, loadRequestDiscussionsForRequest)
        .catch((error: unknown) => {
          console.error('Loading request discussion references failed', error)
          return null
        })
      : null
    const discussionReferences: RequestChangesDiscussionReferences = {
      commitKey: query?.key ?? null,
      page,
    }
    return { discussionReferences, revisions }
  })

const loadRevisionDiff = createServerFn({ method: 'GET' })
  .validator(parseLoadRequestRevisionDiffInput)
  .handler(({ data }) => loadRequestRevisionCommitFileDiffForRequest(data, getRequest().signal))

const loadDiscussions = createServerFn({ method: 'GET' })
  .validator(parseLoadDiscussionsInput)
  .handler(({ data }) => loadDiscussionReferencePage(data, loadRequestDiscussionsForRequest))

const loadDiffForView = (
  data: LoadRequestRevisionCommitInput & { path: string },
  signal?: AbortSignal,
) => loadRevisionDiff({ data, signal })
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
    const replay = takePinnedChangesReplay(input)
    if (replay) return replay
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
    const replay = rememberPinnedChangesReplay(
      {
        ...requestParams,
        commit_oid: changes.pin.commit,
        revision_id: changes.pin.revision,
      },
      changes,
    )
    void navigate({
      params,
      replace: true,
      resetScroll: false,
      search: (current) => ({ ...current, ...changes.pin }),
      to: '/$owner/$repo/requests/$requestId/changes',
    }).then(
      () => forgetPinnedChangesReplay(replay),
      () => forgetPinnedChangesReplay(replay),
    )
  }, [changes, navigate, params, requestParams, search.revision])

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

function rememberPinnedChangesReplay(
  input: LoadRequestRevisionsInput,
  data: ChangesLoaderData,
) {
  if (typeof window === 'undefined') return null
  const replay = { data, key: changesSelectionKey(input) }
  pinnedChangesReplay.current = replay
  return replay
}

function takePinnedChangesReplay(input: LoadRequestRevisionsInput) {
  if (typeof window === 'undefined') return null
  const replay = pinnedChangesReplay.current
  if (!replay || replay.key !== changesSelectionKey(input)) return null
  pinnedChangesReplay.current = null
  return replay.data
}

function forgetPinnedChangesReplay(replay: PinnedChangesReplay | null) {
  if (pinnedChangesReplay.current === replay) pinnedChangesReplay.current = null
}

function changesSelectionKey(input: LoadRequestRevisionsInput) {
  return [
    input.owner,
    input.repo,
    input.request_id,
    input.revision_id ?? '',
    input.commit_oid ?? '',
  ].join('\0')
}
