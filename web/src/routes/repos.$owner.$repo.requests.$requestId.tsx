import { HttpError } from '@/api/client'
import {
  addRequestEditorForRequest,
  commentRequestForRequest,
  deleteRequestForRequest,
  loadRepoLiveStateForRequest,
  loadRequestChangesForRequest,
  loadRequestFileDiffForRequest,
  loadRequestForRequest,
  markRequestNeedsResponseForRequest,
  mergeRequestForRequest,
  parseAddRequestEditorInput,
  parseCommentRequestInput,
  parseMergeRequestInput,
  parseNeedsResponseInput,
  parseRequestParams,
  parseRemoveRequestEditorInput,
  parseResolveRequestInput,
  parseRespondRequestInput,
  removeRequestEditorForRequest,
  resolveRequestForRequest,
  respondToRequestForRequest,
} from '@/api/repos'
import {
  RequestDetailPage,
  RequestUnavailablePage,
} from '@/features/requests/request-detail-page'
import {
  displayRouteFilePath,
  parseRouteFileSearch,
  routeErrorMessage,
  selectedRouteFilePath,
} from '@/lib/route-file'
import { createFileRoute } from '@tanstack/react-router'
import { useNavigate } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const loadRequestPage = createServerFn({ method: 'GET' })
  .validator((data: RequestPageInput) => data)
  .handler(async ({ data }) => {
    const [live, detail] = await Promise.all([
      loadRepoLiveStateForRequest(data),
      loadOptionalRequestForRequest(data),
    ])
    let changes = null
    let changesError = null
    if (detail && data.view === 'changes') {
      try {
        changes = await loadRequestChangesForRequest(data)
      } catch (error) {
        changesError = routeErrorMessage(error, 'Request changes are unavailable.')
      }
    }
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
        selectedDiffError = routeErrorMessage(error, 'File diff is unavailable.')
      }
    }

    return {
      changes,
      changesError,
      detail,
      live,
      selectedDiff,
      selectedDiffError,
      selectedPath,
      view: data.view,
    }
  })

const commentRequest = createServerFn({ method: 'POST' })
  .validator(parseCommentRequestInput)
  .handler(({ data }) => commentRequestForRequest(data))

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

const addRequestEditor = createServerFn({ method: 'POST' })
  .validator(parseAddRequestEditorInput)
  .handler(({ data }) => addRequestEditorForRequest(data))

const removeRequestEditor = createServerFn({ method: 'POST' })
  .validator(parseRemoveRequestEditorInput)
  .handler(({ data }) => removeRequestEditorForRequest(data))

export const Route = createFileRoute('/repos/$owner/$repo/requests/$requestId')({
  validateSearch: parseRequestReviewSearch,
  loaderDeps: ({ search }) => search,
  loader: ({ deps: search, params }) =>
    loadRequestPage({
      data: {
        ...requestParamsForRoute(params),
        ...search,
        view: search.view ?? 'overview',
      },
    }),
  component: RequestRoute,
})

function RequestRoute() {
  const params = Route.useParams()
  const {
    changes,
    changesError,
    detail,
    live,
    selectedDiff,
    selectedDiffError,
    selectedPath,
    view,
  } = Route.useLoaderData()
  const navigate = useNavigate({ from: Route.fullPath })
  const requestParams = {
    owner: params.owner,
    repo: params.repo,
  }

  if (!detail) {
    return <RequestUnavailablePage params={requestParams} />
  }

  return (
    <RequestDetailPage
      addRequestEditor={(data) => addRequestEditor({ data })}
      commentRequest={(data) => commentRequest({ data })}
      changes={changes}
      changesError={changesError}
      detail={detail}
      deleteRequest={(data) => deleteRequest({ data })}
      live={live}
      markNeedsResponse={(data) => markNeedsResponse({ data })}
      mergeRequest={(data) => mergeRequest({ data })}
      params={requestParams}
      onSelectFile={(path) => {
        void navigate({
          search: { file: displayRouteFilePath(path), view: 'changes' },
        })
      }}
      onViewChange={(view) => {
        void navigate({ search: { file: undefined, view } })
      }}
      removeRequestEditor={(data) => removeRequestEditor({ data })}
      resolveRequest={(data) => resolveRequest({ data })}
      respondToRequest={(data) => respondToRequest({ data })}
      selectedDiff={selectedDiff}
      selectedDiffError={selectedDiffError}
      selectedPath={selectedPath}
      view={view}
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

type RequestReviewView = 'overview' | 'changes' | 'activity'
type RequestReviewSearch = { file?: string; view?: RequestReviewView }
type RequestPageInput = ReturnType<typeof requestParamsForRoute> & {
  file?: string
  view: RequestReviewView
}

function parseRequestReviewSearch(
  search: Record<string, unknown>,
): RequestReviewSearch {
  const view: RequestReviewView | undefined =
    search.view === 'overview' ||
    search.view === 'changes' ||
    search.view === 'activity'
      ? search.view
      : undefined
  return {
    file: view === 'changes' ? parseRouteFileSearch(search.file) : undefined,
    view,
  }
}
