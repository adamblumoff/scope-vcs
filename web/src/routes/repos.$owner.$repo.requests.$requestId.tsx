import { createApiClient, HttpError } from '@/api/client'
import type { AccountSession } from '@/api/types'
import {
  ApiRouteTemplates,
  buildApiPath,
} from '@/api/types.generated'
import {
  deleteRequestForRequest,
  loadRequestChangesForRequest,
  loadRequestFileDiffForRequest,
  loadRequestForRequest,
  markRequestNeedsResponseForRequest,
  mergeRequestForRequest,
  parseMergeRequestInput,
  parseNeedsResponseInput,
  parseRequestParams,
  parseResolveRequestInput,
  parseRespondRequestInput,
  resolveRequestForRequest,
  respondToRequestForRequest,
} from '@/api/repos'
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
  reopenRequestDiscussionForRequest,
  type RequestDiscussionActionInput,
  resolveRequestDiscussionForRequest,
  updateRequestDescriptionForRequest,
  type UpdateDescriptionInput,
} from '@/features/requests/request-discussion-api'
import type {
  RequestDiscussionFilter,
  RequestDiscussionSort,
} from '@/features/requests/request-discussion-types'
import {
  RequestDetailPage,
  RequestUnavailablePage,
} from '@/features/requests/request-detail-page'
import type {
  RequestReviewView,
} from '@/features/requests/request-review-navigation'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import {
  displayRouteFilePath,
  parseRouteFileSearch,
  routeErrorMessage,
  selectedRouteFilePath,
} from '@/lib/route-file'
import {
  createFileRoute,
  useNavigate,
} from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const loadRequestPage = createServerFn({ method: 'GET' })
  .validator((data: RequestPageInput) => data)
  .handler(async ({ data }) => {
    const discussionPromise =
      data.view === 'discussion'
        ? loadOptionalSelectedRequestResource(() =>
            loadRequestDiscussionsForRequest({
              ...data,
              sort: data.discussionSort,
              status: data.discussionFilter,
            }),
          )
        : Promise.resolve(null)
    const activityPromise =
      data.view === 'activity'
        ? loadOptionalSelectedRequestResource(() =>
            loadRequestActivityForRequest(data),
          )
        : Promise.resolve(null)
    const changesPromise =
      data.view === 'changes'
        ? loadChangesWithError(data)
        : Promise.resolve({ changes: null, changesError: null })
    const [detail, account, discussionPage, activity, loadedChanges] =
      await Promise.all([
        loadOptionalRequestForRequest(data),
        loadOptionalAccountSession(),
        discussionPromise,
        activityPromise,
        changesPromise,
      ])
    if (!detail) return unavailablePage(data)

    const { changes, changesError } = loadedChanges

    const selectedPath = changes
      ? selectedRouteFilePath(changes.files, data.file)
      : null
    let selectedDiff = null
    let selectedDiffError = null
    if (selectedPath) {
      try {
        selectedDiff = await loadRequestFileDiffForRequest({
          ...data,
          path: selectedPath,
        })
      } catch (error) {
        selectedDiffError = routeErrorMessage(
          error,
          'File diff is unavailable.',
        )
      }
    }

    return {
      activity,
      account,
      changes,
      changesError,
      detail,
      discussionFilter: data.discussionFilter,
      discussionPage,
      discussionSort: data.discussionSort,
      selectedDiff,
      selectedDiffError,
      selectedPath,
      view: data.view,
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
  .validator(
    (data: ReturnType<typeof requestParamsForRoute> & { after: number }) =>
      data,
  )
  .handler(({ data }) => loadRequestDiscussionChangesForRequest(data))

const createDiscussion = createServerFn({ method: 'POST' })
  .validator((data: CreateDiscussionInput) => data)
  .handler(({ data }) => createRequestDiscussionForRequest(data))

const createReply = createServerFn({ method: 'POST' })
  .validator((data: CreateReplyInput) => data)
  .handler(({ data }) => createRequestDiscussionReplyForRequest(data))

const resolveDiscussion = createServerFn({ method: 'POST' })
  .validator((data: RequestDiscussionActionInput) => data)
  .handler(({ data }) => resolveRequestDiscussionForRequest(data))

const reopenDiscussion = createServerFn({ method: 'POST' })
  .validator((data: RequestDiscussionActionInput) => data)
  .handler(({ data }) => reopenRequestDiscussionForRequest(data))

const reopenAndReply = createServerFn({ method: 'POST' })
  .validator((data: CreateReplyInput) => data)
  .handler(({ data }) => reopenAndReplyToRequestDiscussionForRequest(data))

const markDiscussionRead = createServerFn({ method: 'POST' })
  .validator((data: MarkDiscussionReadInput) => data)
  .handler(({ data }) => markRequestDiscussionReadForRequest(data))

const updateDescription = createServerFn({ method: 'POST' })
  .validator((data: UpdateDescriptionInput) => data)
  .handler(({ data }) => updateRequestDescriptionForRequest(data))

const markNeedsResponse = createServerFn({ method: 'POST' })
  .validator(parseNeedsResponseInput)
  .handler(({ data }) => markRequestNeedsResponseForRequest(data))

const respondToRequest = createServerFn({ method: 'POST' })
  .validator(parseRespondRequestInput)
  .handler(({ data }) => respondToRequestForRequest(data))

const resolveRequest = createServerFn({ method: 'POST' })
  .validator(parseResolveRequestInput)
  .handler(({ data }) => resolveRequestForRequest(data))

const mergeRequest = createServerFn({ method: 'POST' })
  .validator(parseMergeRequestInput)
  .handler(({ data }) => mergeRequestForRequest(data))

const deleteRequest = createServerFn({ method: 'POST' })
  .validator(parseRequestParams)
  .handler(({ data }) => deleteRequestForRequest(data))

export const Route = createFileRoute('/repos/$owner/$repo/requests/$requestId')({
  validateSearch: parseRequestReviewSearch,
  loaderDeps: ({ search }) => search,
  loader: ({ deps: search, params }) =>
    loadRequestPage({
      data: {
        ...requestParamsForRoute(params),
        discussionFilter:
          search.discussionStatus === 'all' ? 'All' : 'Open',
        discussionSort:
          search.discussionSort === 'newest' ? 'Newest' : 'Recent',
        file: search.file,
        view: search.view ?? 'discussion',
      },
    }),
  component: RequestRoute,
})

function RequestRoute() {
  const params = Route.useParams()
  const page = Route.useLoaderData()
  const live = useRepoLayout()
  const navigate = useNavigate({ from: Route.fullPath })
  const requestParams = {
    owner: params.owner,
    repo: params.repo,
  }

  if (!page.detail) {
    return <RequestUnavailablePage params={requestParams} />
  }

  return (
    <RequestDetailPage
      activity={page.activity}
      actor={page.account?.user ?? null}
      changes={page.changes}
      changesError={page.changesError}
      createDiscussion={(data) => createDiscussion({ data })}
      createReply={(data) => createReply({ data })}
      deleteRequest={(data) => deleteRequest({ data })}
      detail={page.detail}
      discussionFilter={page.discussionFilter}
      discussionPage={page.discussionPage}
      discussionSort={page.discussionSort}
      live={live}
      loadActivity={(data) => loadActivity({ data })}
      loadDiscussions={(data) => loadDiscussions({ data })}
      loadDiscussionChanges={(data) => loadDiscussionChanges({ data })}
      loadReplies={(data) => loadReplies({ data })}
      markDiscussionRead={(data) => markDiscussionRead({ data })}
      markNeedsResponse={(data) => markNeedsResponse({ data })}
      mergeRequest={(data) => mergeRequest({ data })}
      onDiscussionQueryChange={({ filter, sort }) => {
        void navigate({
          params,
          resetScroll: false,
          search: (previous) => ({
            ...previous,
            discussionSort: sort === 'Newest' ? 'newest' : undefined,
            discussionStatus: filter === 'All' ? 'all' : undefined,
            file: undefined,
            view: 'discussion',
          }),
          to: Route.fullPath,
        })
      }}
      onSelectFile={(path) => {
        void navigate({
          params,
          resetScroll: false,
          search: (previous) => ({
            ...previous,
            discussionSort: undefined,
            discussionStatus: undefined,
            file: displayRouteFilePath(path),
            view: 'changes',
          }),
          to: Route.fullPath,
        })
      }}
      onViewChange={(view) => {
        void navigate({
          params,
          resetScroll: false,
          search: (previous) => ({
            ...previous,
            discussionSort:
              view === 'discussion' && page.discussionSort === 'Newest'
                ? 'newest'
                : undefined,
            discussionStatus:
              view === 'discussion' && page.discussionFilter === 'All'
                ? 'all'
                : undefined,
            file: undefined,
            view,
          }),
          to: Route.fullPath,
        })
      }}
      params={requestParams}
      reopenAndReply={(data) => reopenAndReply({ data })}
      reopenDiscussion={(data) => reopenDiscussion({ data })}
      resolveDiscussion={(data) => resolveDiscussion({ data })}
      resolveRequest={(data) => resolveRequest({ data })}
      respondToRequest={(data) => respondToRequest({ data })}
      selectedDiff={page.selectedDiff}
      selectedDiffError={page.selectedDiffError}
      selectedPath={page.selectedPath}
      updateDescription={(data) => updateDescription({ data })}
      view={page.view}
    />
  )
}

function requestParamsForRoute(params: {
  owner: string
  repo: string
  requestId: string
}) {
  return {
    owner: params.owner,
    repo: params.repo,
    request_id: params.requestId,
  }
}

async function loadOptionalRequestForRequest(
  data: ReturnType<typeof requestParamsForRoute>,
) {
  try {
    return await loadRequestForRequest(data)
  } catch (error) {
    if (error instanceof HttpError && [403, 404].includes(error.status)) {
      return null
    }
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

async function loadOptionalSelectedRequestResource<T>(
  load: () => Promise<T>,
) {
  try {
    return await load()
  } catch (error) {
    if (error instanceof HttpError && [403, 404].includes(error.status)) {
      return null
    }
    throw error
  }
}

async function loadChangesWithError(
  data: ReturnType<typeof requestParamsForRoute>,
) {
  try {
    return {
      changes: await loadRequestChangesForRequest(data),
      changesError: null,
    }
  } catch (error) {
    return {
      changes: null,
      changesError: routeErrorMessage(
        error,
        'Request changes are unavailable.',
      ),
    }
  }
}

function unavailablePage(data: RequestPageInput) {
  return {
    activity: null,
    account: null,
    changes: null,
    changesError: null,
    detail: null,
    discussionFilter: data.discussionFilter,
    discussionPage: null,
    discussionSort: data.discussionSort,
    selectedDiff: null,
    selectedDiffError: null,
    selectedPath: null,
    view: data.view,
  }
}

type RequestReviewSearch = {
  discussionSort?: 'newest'
  discussionStatus?: 'all'
  file?: string
  view?: RequestReviewView
}

type RequestPageInput = ReturnType<typeof requestParamsForRoute> & {
  discussionFilter: RequestDiscussionFilter
  discussionSort: RequestDiscussionSort
  file?: string
  view: RequestReviewView
}

function parseRequestReviewSearch(
  search: Record<string, unknown>,
): RequestReviewSearch {
  const view: RequestReviewView | undefined =
    search.view === 'discussion' ||
    search.view === 'changes' ||
    search.view === 'activity'
      ? search.view
      : undefined
  return {
    discussionSort:
      search.discussionSort === 'newest' ? 'newest' : undefined,
    discussionStatus:
      search.discussionStatus === 'all' ? 'all' : undefined,
    file: view === 'changes' ? parseRouteFileSearch(search.file) : undefined,
    view,
  }
}
