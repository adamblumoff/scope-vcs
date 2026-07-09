import { HttpError } from '@/api/client'
import {
  addRequestEditorForRequest,
  commentRequestForRequest,
  deleteRequestForRequest,
  loadRepoLiveStateForRequest,
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
import { createFileRoute } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const loadRequestPage = createServerFn({ method: 'GET' })
  .validator(parseRequestParams)
  .handler(async ({ data }) => {
    const [live, detail] = await Promise.all([
      loadRepoLiveStateForRequest(data),
      loadOptionalRequestForRequest(data),
    ])

    return { detail, live }
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
  loader: ({ params }) =>
    loadRequestPage({
      data: requestParamsForRoute(params),
    }),
  component: RequestRoute,
})

function RequestRoute() {
  const params = Route.useParams()
  const { detail, live } = Route.useLoaderData()
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
      detail={detail}
      deleteRequest={(data) => deleteRequest({ data })}
      live={live}
      markNeedsResponse={(data) => markNeedsResponse({ data })}
      mergeRequest={(data) => mergeRequest({ data })}
      params={requestParams}
      removeRequestEditor={(data) => removeRequestEditor({ data })}
      resolveRequest={(data) => resolveRequest({ data })}
      respondToRequest={(data) => respondToRequest({ data })}
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
