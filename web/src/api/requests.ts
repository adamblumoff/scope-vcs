import { createApiClient } from '@/api/client'
import { prerenderReviewFileDiff } from '@/features/review/review-file-diff-prerender'
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
import type { LoadRequestQueueInput } from './request-queue-input'


export async function loadRequestQueueForRequest(
  data: LoadRequestQueueInput,
): Promise<RequestList> {
  return createApiClient().get<RequestList>(requestQueuePath(data), {
    auth: 'optional',
  })
}


export async function loadRequestForRequest(
  data: RequestParams,
): Promise<RequestDetail> {
  return createApiClient().get<RequestDetail>(requestPath(data), {
    auth: 'optional',
  })
}

export type RateRequestInput = RequestParams & {
  score: number
  reason: string
}

export async function loadRequestRatingsForRequest(
  data: RequestParams,
): Promise<RequestRatings> {
  return createApiClient().get<RequestRatings>(
    requestRoute(ApiRouteTemplates.repoRequestRatings, data),
    { auth: 'optional' },
  )
}

export async function rateRequestForRequest(
  data: RateRequestInput,
): Promise<RequestRating> {
  return createApiClient().post<RequestRating>(
    requestRoute(ApiRouteTemplates.repoRequestRatings, data),
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
  return createApiClient().get<RequestRevisions>(
    search.size > 0 ? `${path}?${search}` : path,
    { auth: 'optional' },
  )
}

export type LoadRequestRevisionCommitInput = RequestParams & {
  commit_oid: string
  revision_id: string
}

export async function loadRequestRevisionCommitFileDiffForRequest(
  data: LoadRequestRevisionCommitInput & { path: string },
): Promise<ReviewFileDiff> {
  const diff = await createApiClient().get<ReviewFileDiff>(
    `${requestRevisionCommitRoute(ApiRouteTemplates.repoRequestRevisionCommitFileDiff, data)}?path=${encodeURIComponent(data.path)}`,
    { auth: 'optional' },
  )

  return {
    ...diff,
    prerenderedHtml: await prerenderReviewFileDiff(diff),
  }
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
