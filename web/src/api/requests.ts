import { createApiClient } from '@/api/client'
import { renderReviewFileDiff } from '@/features/review/review-file-diff-prerender'
import type {
  RequestDetail,
  RequestList,
  RequestRating,
  RequestRatings,
  RequestRevisions,
  ReviewFileDiff,
  RequestParams,
} from './types'
import { ApiRouteTemplates, buildApiPath } from './types.generated'
import { apiValidators } from './validators.generated'
import type { LoadRequestQueueInput } from './request-queue-input'

export async function loadRequestQueueForRequest(
  data: LoadRequestQueueInput,
): Promise<RequestList> {
  return createApiClient().get(
    requestQueuePath(data),
    apiValidators.RequestListResponse,
    { auth: 'optional' },
  )
}

export async function loadRequestForRequest(
  data: RequestParams,
): Promise<RequestDetail> {
  return createApiClient().get(
    requestPath(data),
    apiValidators.RequestDetailResponse,
    { auth: 'optional' },
  )
}

export type RateRequestInput = RequestParams & {
  score: number
  reason: string
}

export async function loadRequestRatingsForRequest(
  data: RequestParams,
): Promise<RequestRatings> {
  return createApiClient().get(
    requestRoute(ApiRouteTemplates.repoRequestRatings, data),
    apiValidators.RequestRatingsResponse,
    { auth: 'optional' },
  )
}

export async function rateRequestForRequest(
  data: RateRequestInput,
): Promise<RequestRating> {
  return createApiClient().post(
    requestRoute(ApiRouteTemplates.repoRequestRatings, data),
    apiValidators.RequestRatingResponse,
    {
      auth: 'required',
      body: { reason: data.reason, score: data.score },
    },
  )
}

export async function loadRequestRevisionsForRequest(
  data: RequestParams & { commit_oid?: string; revision_id?: string },
): Promise<RequestRevisions> {
  const search = new URLSearchParams()
  if (data.revision_id) search.set('revision', data.revision_id)
  if (data.commit_oid) search.set('commit', data.commit_oid)
  const path = requestRoute(ApiRouteTemplates.repoRequestRevisions, data)
  return createApiClient().get(
    search.size > 0 ? `${path}?${search}` : path,
    apiValidators.RequestRevisionListResponse,
    { auth: 'optional' },
  )
}

export type LoadRequestRevisionCommitInput = RequestParams & {
  commit_oid: string
  revision_id: string
}

export async function loadRequestRevisionCommitFileDiffForRequest(
  data: LoadRequestRevisionCommitInput & { path: string },
  signal?: AbortSignal,
): Promise<ReviewFileDiff> {
  const path = requestRevisionCommitRoute(
    ApiRouteTemplates.repoRequestRevisionCommitFileDiff,
    data,
  )
  const diff = await createApiClient().get(
    `${path}?path=${encodeURIComponent(data.path)}`,
    apiValidators.ReviewFileDiffResponse,
    { auth: 'optional', signal },
  )

  return renderReviewFileDiff(diff, signal)
}

function requestQueuePath(data: LoadRequestQueueInput) {
  const path = buildApiPath(ApiRouteTemplates.repoRequestQueue, {
    owner: data.owner,
    repo: data.repo,
  })
  const search = new URLSearchParams({ section: data.section })
  if (data.cursor) {
    search.set('cursor', data.cursor)
  }
  if (data.search) {
    search.set('search', data.search)
  }
  return `${path}?${search}`
}

function requestPath(data: RequestParams) {
  return requestRoute(ApiRouteTemplates.repoRequest, data)
}

function requestRoute(template: string, data: RequestParams) {
  return buildApiPath(template, {
    owner: data.owner,
    repo: data.repo,
    request_id: data.request_id,
  })
}

function requestRevisionCommitRoute(
  template: string,
  data: LoadRequestRevisionCommitInput,
) {
  return buildApiPath(template, {
    commit_oid: data.commit_oid,
    owner: data.owner,
    repo: data.repo,
    request_id: data.request_id,
    revision_id: data.revision_id,
  })
}
