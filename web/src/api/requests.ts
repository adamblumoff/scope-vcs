import { createApiClient } from '@/api/client'
import type {
  DeleteRequestInput,
  MergeRequestInput,
  NeedsResponseInput,
  RepoParams,
  RequestDetail,
  RequestDelete,
  RequestChangeBlockFiles,
  RequestList,
  RequestMutation,
  ReviewFileDiff,
  RequestParams,
  ResolveRequestInput,
  RespondRequestInput,
} from './types'
import { ApiRouteTemplates, buildApiPath } from './types.generated'
import { parseRepoParams } from './repo-params'

export {
  parseMergeRequestInput,
  parseNeedsResponseInput,
  parseRequestParams,
  parseResolveRequestInput,
  parseRespondRequestInput,
} from './request-inputs'

export async function loadRequestsForRequest(
  data: LoadRequestsInput,
): Promise<RequestList> {
  return createApiClient().get<RequestList>(requestCollectionPath(data), {
    auth: 'optional',
  })
}

export type LoadRequestsInput = RepoParams & {
  cursor?: string | null
}

export function parseLoadRequestsInput(input: unknown): LoadRequestsInput {
  const data = input as Partial<LoadRequestsInput> | null
  const params = parseRepoParams(data)
  const cursor = typeof data?.cursor === 'string' ? data.cursor.trim() : ''

  return {
    ...params,
    cursor: cursor || null,
  }
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

export async function markRequestNeedsResponseForRequest(
  data: NeedsResponseInput,
): Promise<RequestMutation> {
  return createApiClient().post<RequestMutation>(
    requestRoute(ApiRouteTemplates.repoRequestNeedsResponse, data),
    {
      auth: 'required',
      body: { body: data.body },
    },
  )
}

export async function respondToRequestForRequest(
  data: RespondRequestInput,
): Promise<RequestMutation> {
  return createApiClient().post<RequestMutation>(
    requestRoute(ApiRouteTemplates.repoRequestRespond, data),
    { auth: 'required', body: { body: data.body } },
  )
}

export async function resolveRequestForRequest(
  data: ResolveRequestInput,
): Promise<RequestMutation> {
  return createApiClient().post<RequestMutation>(
    requestRoute(ApiRouteTemplates.repoRequestResolve, data),
    {
      auth: 'required',
      body: {
        body: data.body,
        disposition: data.disposition,
      },
    },
  )
}

export async function mergeRequestForRequest(
  data: MergeRequestInput,
): Promise<RequestMutation> {
  return createApiClient().post<RequestMutation>(
    requestRoute(ApiRouteTemplates.repoRequestMerge, data),
    {
      auth: 'required',
      body: {
        body: data.body,
        expected_head_oid: data.expected_head_oid,
        expected_main_oid: data.expected_main_oid,
      },
    },
  )
}

export async function deleteRequestForRequest(
  data: DeleteRequestInput,
): Promise<RequestDelete> {
  return createApiClient().delete<RequestDelete>(requestPath(data), {
    auth: 'required',
  })
}

function requestCollectionPath(data: LoadRequestsInput) {
  const path = buildApiPath(ApiRouteTemplates.repoRequests, {
    owner: data.owner,
    repo: data.repo,
  })
  const search = new URLSearchParams()
  if (data.cursor) {
    search.set('cursor', data.cursor)
  }
  const query = search.toString()
  return query ? `${path}?${query}` : path
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
