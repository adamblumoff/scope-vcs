import { createApiClient, HttpError } from '@/api/client'
import type { AccountSession } from '@/api/types'
import { ApiRouteTemplates, buildApiPath } from '@/api/types.generated'
import { loadRequestForRequest } from '@/api/repos'
import {
  type LoadRequestRevisionCommitInput,
  loadRequestRevisionCommitFileDiffForRequest,
  loadRequestRevisionCommitForRequest,
  loadRequestRevisionsForRequest,
  loadRequestRatingsForRequest,
  rateRequestForRequest,
  type RateRequestInput,
} from '@/api/requests'
import {
  type RequestActionCommand,
  type RequestActionInput,
  performRequestActionForRequest,
} from '@/features/requests/request-actions-api'
import {
  createRequestDiscussionForRequest,
  createRequestDiscussionReplyForRequest,
  type CreateDiscussionInput,
  type CreateReplyInput,
  type LoadActivityInput,
  loadRequestActivityForRequest,
  loadRequestDiscussionRepliesForRequest,
  loadRequestDiscussionChangesForRequest,
  loadRequestDiscussionsForRequest,
  type LoadDiscussionsInput,
  type LoadRepliesInput,
  markRequestDiscussionReadForRequest,
  type MarkDiscussionReadInput,
  reopenAndReplyToRequestDiscussionForRequest,
  type RequestDiscussionActionInput,
  resolveRequestDiscussionForRequest,
  updateRequestDescriptionForRequest,
  type UpdateDescriptionInput,
} from '@/features/requests/request-discussion-api'
import {
  RequestDetailPage,
  RequestUnavailablePage,
} from '@/features/requests/request-detail-page'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import type { RequestChangesSearch } from '@/features/requests/request-changes-workbench'
import { includeFocusedDiscussion } from '@/features/requests/request-discussion-model'
import { parseRouteFileSearch } from '@/lib/route-file'
import { createFileRoute, useMatchRoute, useRouter } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { useCallback, useMemo } from 'react'

const loadRequestPage = createServerFn({ method: 'GET' })
  .validator((data: ReturnType<typeof requestPageInput>) => data)
  .handler(async ({ data }) => {
    const requestParams = {
      owner: data.owner,
      repo: data.repo,
      request_id: data.request_id,
    }
    const [detail, account, discussionPage, focusedDiscussionPage, ratings, revisions] = await Promise.all([
      loadOptionalRequestForRequest(requestParams),
      loadOptionalAccountSession(),
      loadOptionalSelectedRequestResource(() => loadRequestDiscussionsForRequest(requestParams)),
      data.discussion
        ? loadOptionalSelectedRequestResource(() => loadRequestDiscussionsForRequest({
            ...requestParams,
            discussion_id: data.discussion,
          }))
        : Promise.resolve(null),
      loadOptionalSelectedRequestResource(() => loadRequestRatingsForRequest(requestParams)),
      data.include_revisions
        ? loadOptionalRequestRevisions({
            ...requestParams,
            commit_oid: data.commit,
            revision_id: data.revision,
          })
        : Promise.resolve(null),
    ])
    return {
      account,
      detail,
      discussionPage: includeFocusedDiscussion(discussionPage, focusedDiscussionPage),
      ratings,
      revisions,
    }
  })

const loadDiscussions = createServerFn({ method: 'GET' })
  .validator((data: LoadDiscussionsInput) => data)
  .handler(({ data }) => loadRequestDiscussionsForRequest(data))

const loadActivity = createServerFn({ method: 'GET' })
  .validator((data: LoadActivityInput) => data)
  .handler(({ data }) => loadRequestActivityForRequest(data))

const loadReplies = createServerFn({ method: 'GET' })
  .validator((data: LoadRepliesInput) => data)
  .handler(({ data }) => loadRequestDiscussionRepliesForRequest(data))

const loadDiscussionChanges = createServerFn({ method: 'GET' })
  .validator((data: ReturnType<typeof requestParamsForRoute> & { after: number }) => data)
  .handler(({ data }) => loadRequestDiscussionChangesForRequest(data))

const loadRevisionCommit = createServerFn({ method: 'GET' })
  .validator((data: LoadRequestRevisionCommitInput) => data)
  .handler(({ data }) => loadRequestRevisionCommitForRequest(data))

const loadRevisionDiff = createServerFn({ method: 'GET' })
  .validator((data: LoadRequestRevisionCommitInput & { path: string }) => data)
  .handler(({ data }) => loadRequestRevisionCommitFileDiffForRequest(data))

const createDiscussion = createServerFn({ method: 'POST' })
  .validator((data: CreateDiscussionInput) => data)
  .handler(({ data }) => createRequestDiscussionForRequest(data))

const createReply = createServerFn({ method: 'POST' })
  .validator((data: CreateReplyInput) => data)
  .handler(({ data }) => createRequestDiscussionReplyForRequest(data))

const resolveDiscussion = createServerFn({ method: 'POST' })
  .validator((data: RequestDiscussionActionInput) => data)
  .handler(({ data }) => resolveRequestDiscussionForRequest(data))

const reopenAndReply = createServerFn({ method: 'POST' })
  .validator((data: CreateReplyInput) => data)
  .handler(({ data }) => reopenAndReplyToRequestDiscussionForRequest(data))

const markDiscussionRead = createServerFn({ method: 'POST' })
  .validator((data: MarkDiscussionReadInput) => data)
  .handler(({ data }) => markRequestDiscussionReadForRequest(data))

const updateDescription = createServerFn({ method: 'POST' })
  .validator((data: UpdateDescriptionInput) => data)
  .handler(({ data }) => updateRequestDescriptionForRequest(data))

const runRequestAction = createServerFn({ method: 'POST' })
  .validator((data: RequestActionInput) => data)
  .handler(({ data }) => performRequestActionForRequest(data))

const rateRequest = createServerFn({ method: 'POST' })
  .validator((data: RateRequestInput) => data)
  .handler(({ data }) => rateRequestForRequest(data))

export const Route = createFileRoute('/$owner/$repo/requests/$requestId')({
  validateSearch: parseRequestDetailSearch,
  loaderDeps: ({ search }) => ({
    commit: search.commit,
    discussion: search.discussion,
    revision: search.revision,
  }),
  loader: ({ deps, location, params }) => loadRequestPage({
    data: requestPageInput(
      params,
      location.pathname.endsWith('/changes'),
      deps,
    ),
  }),
  component: RequestRoute,
})

function RequestRoute() {
  const params = Route.useParams()
  const page = Route.useLoaderData()
  const live = useRepoLayout()
  const matchRoute = useMatchRoute()
  const router = useRouter()
  const navigate = Route.useNavigate()
  const search = Route.useSearch()
  const activeView = matchRoute({
    params,
    to: '/$owner/$repo/requests/$requestId/changes',
  }) ? 'changes' : 'discussion'
  const repoParams = useMemo(
    () => ({ owner: params.owner, repo: params.repo }),
    [params.owner, params.repo],
  )
  const requestParams = useMemo(
    () => requestParamsForRoute({
      owner: params.owner,
      repo: params.repo,
      requestId: params.requestId,
    }),
    [params.owner, params.repo, params.requestId],
  )
  const performAction = useCallback(async (command: RequestActionCommand) => {
    const result = await runRequestAction({ data: { ...requestParams, ...command } })
    try {
      if (result.deleted) {
        await navigate({ params: repoParams, to: '/$owner/$repo/requests' })
      } else {
        await router.invalidate()
      }
      return result
    } catch {
      return {
        ...result,
        synchronizationError: 'The update completed, but the latest request state could not be reloaded. Refresh this page.',
      }
    }
  }, [navigate, repoParams, requestParams, router])
  const rateParticipant = useCallback(async (input: RateRequestInput) => {
    const rating = await rateRequest({ data: input })
    await router.invalidate()
    return rating
  }, [router])

  if (!page.detail || !page.discussionPage || !page.ratings) {
    return <RequestUnavailablePage params={repoParams} />
  }

  return (
    <RequestDetailPage
      account={page.account}
      activeView={activeView}
      createDiscussion={(data) => createDiscussion({ data })}
      createReply={(data) => createReply({ data })}
      detail={page.detail}
      discussionPage={page.discussionPage}
      live={live}
      loadActivity={() => loadActivity({ data: requestParams })}
      loadRevisionCommit={(data) => loadRevisionCommit({ data })}
      loadRevisionDiff={(data) => loadRevisionDiff({ data })}
      loadDiscussions={(data) => loadDiscussions({ data })}
      loadDiscussionChanges={(data) => loadDiscussionChanges({ data })}
      loadReplies={(data) => loadReplies({ data })}
      markDiscussionRead={(data) => markDiscussionRead({ data })}
      onChangesSearchChange={(nextSearch) => {
        void navigate({
          params,
          replace: true,
          resetScroll: false,
          search: nextSearch,
          to: '/$owner/$repo/requests/$requestId/changes',
        })
      }}
      params={repoParams}
      performAction={performAction}
      reopenAndReply={(data) => reopenAndReply({ data })}
      revisions={page.revisions}
      ratings={page.ratings}
      rateRequest={rateParticipant}
      resolveDiscussion={(data) => resolveDiscussion({ data })}
      updateDescription={(data) => updateDescription({ data })}
      search={search}
    />
  )
}

export type RequestDetailSearch = RequestChangesSearch & {
  discussion?: string
}

function parseRequestDetailSearch(
  search: Record<string, unknown>,
): RequestDetailSearch {
  return {
    commit: searchText(search.commit),
    discussion: searchText(search.discussion),
    path: searchPath(search.path),
    revision: searchText(search.revision),
  }
}

function searchPath(value: unknown) {
  const path = parseRouteFileSearch(value)
  return path ? `/${path}` : undefined
}

function searchText(value: unknown) {
  return typeof value === 'string' && value.trim() ? value.trim() : undefined
}

function requestParamsForRoute(params: { owner: string; repo: string; requestId: string }) {
  return { owner: params.owner, repo: params.repo, request_id: params.requestId }
}

function requestPageInput(
  params: { owner: string; repo: string; requestId: string },
  includeRevisions: boolean,
  selection: { commit?: string; discussion?: string; revision?: string },
) {
  return {
    ...requestParamsForRoute(params),
    ...selection,
    include_revisions: includeRevisions,
  }
}

async function loadOptionalRequestForRequest(data: ReturnType<typeof requestParamsForRoute>) {
  try {
    return await loadRequestForRequest(data)
  } catch (error) {
    if (error instanceof HttpError && [403, 404].includes(error.status)) return null
    throw error
  }
}

async function loadOptionalAccountSession() {
  try {
    return await createApiClient().get<AccountSession>(
      buildApiPath(ApiRouteTemplates.accountSession),
      { auth: 'optional' },
    )
  } catch (error) {
    if (error instanceof HttpError && error.status === 401) return null
    throw error
  }
}

async function loadOptionalSelectedRequestResource<T>(load: () => Promise<T>) {
  try {
    return await load()
  } catch (error) {
    if (error instanceof HttpError && [403, 404].includes(error.status)) return null
    throw error
  }
}

async function loadOptionalRequestRevisions(
  data: ReturnType<typeof requestParamsForRoute> & {
    commit_oid?: string
    revision_id?: string
  },
) {
  try {
    return await loadRequestRevisionsForRequest(data)
  } catch (error) {
    console.error('Loading request revisions failed', error)
    return null
  }
}
