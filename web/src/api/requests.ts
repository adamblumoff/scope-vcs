import { createApiClient } from '@/api/client'
import type {
  RequestDetail,
  RequestChangeBlockFiles,
  RequestList,
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

export async function loadRequestChangeBlockFilesForRequest(
  data: LoadRequestChangeBlockFilesInput,
): Promise<RequestChangeBlockFiles> {
  return createApiClient().get<RequestChangeBlockFiles>(
    requestChangeBlockRoute(ApiRouteTemplates.repoRequestChangeBlockFiles, data),
    { auth: 'optional' },
  )
}

export type LoadRequestChangeBlockFilesInput = RequestParams & {
  block_id: string
}

export async function loadRequestChangeBlockFileDiffForRequest(
  data: RequestParams & { block_id: string; path: string },
): Promise<ReviewFileDiff> {
  return createApiClient().get<ReviewFileDiff>(
    `${requestChangeBlockRoute(ApiRouteTemplates.repoRequestChangeBlockFileDiff, data)}?path=${encodeURIComponent(data.path)}`,
    { auth: 'optional' },
  )
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

function requestChangeBlockRoute(
  template: string,
  data: RequestParams & { block_id: string },
) {
  return buildApiPath(template, {
    block_id: data.block_id,
    owner: data.owner,
    repo: data.repo,
    request_id: data.request_id,
  })
}
