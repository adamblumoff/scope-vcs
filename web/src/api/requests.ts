import { createApiClient } from '@/api/client'
import type {
  DeleteRequestInput,
  MergeRequestInput,
  NeedsResponseInput,
  RepoParams,
  RequestDetail,
  RequestDelete,
  RequestChanges,
  RequestList,
  RequestMutation,
  ReviewFileDiff,
  RequestParams,
  ResolveRequestInput,
  RespondRequestInput,
} from './types'
import { ApiRouteTemplates, buildApiPath } from './types.generated'

export {
  parseMergeRequestInput,
  parseNeedsResponseInput,
  parseRequestParams,
  parseResolveRequestInput,
  parseRespondRequestInput,
} from './request-inputs'

export async function loadRequestsForRequest(
  data: RepoParams,
): Promise<RequestList> {
  return createApiClient().get<RequestList>(requestCollectionPath(data), {
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

export async function loadRequestChangesForRequest(
  data: RequestParams,
): Promise<RequestChanges> {
  return createApiClient().get<RequestChanges>(
    requestRoute(ApiRouteTemplates.repoRequestChanges, data),
    { auth: 'optional' },
  )
}

export async function loadRequestFileDiffForRequest(
  data: RequestParams & { path: string },
): Promise<ReviewFileDiff> {
  return createApiClient().get<ReviewFileDiff>(
    `${requestRoute(ApiRouteTemplates.repoRequestFileDiff, data)}?path=${encodeURIComponent(data.path)}`,
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

function requestCollectionPath(data: RepoParams) {
  return buildApiPath(ApiRouteTemplates.repoRequests, {
    owner: data.owner,
    repo: data.repo,
  })
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
