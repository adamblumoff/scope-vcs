import {
  commentRequestForRequest,
  loadRepoLiveStateForRequest,
  loadRequestForRequest,
  markRequestNeedsResponseForRequest,
  mergeRequestForRequest,
  parseCommentRequestInput,
  parseMergeRequestInput,
  parseNeedsResponseInput,
  parseRequestParams,
  parseResolveRequestInput,
  parseRespondRequestInput,
  resolveRequestForRequest,
  respondToRequestForRequest,
} from '@/api/repos'
import { RequestDetailPage } from '@/features/requests/request-detail-page'
import { createFileRoute } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const loadRequestPage = createServerFn({ method: 'GET' })
  .validator(parseRequestParams)
  .handler(async ({ data }) => {
    const [live, detail] = await Promise.all([
      loadRepoLiveStateForRequest(data),
      loadRequestForRequest(data),
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

  return (
    <RequestDetailPage
      commentRequest={(data) => commentRequest({ data })}
      detail={detail}
      live={live}
      markNeedsResponse={(data) => markNeedsResponse({ data })}
      mergeRequest={(data) => mergeRequest({ data })}
      params={requestParams}
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
