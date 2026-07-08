import { createApiClient } from '@/api/client'
import type {
  CommentRequestInput,
  MergeRequestInput,
  NeedsResponseInput,
  RepoParams,
  RequestDetail,
  RequestList,
  RequestMutation,
  RequestParams,
  ResolveRequestInput,
  RespondRequestInput,
} from './types'

export {
  parseCommentRequestInput,
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

export async function commentRequestForRequest(
  data: CommentRequestInput,
): Promise<RequestMutation> {
  return createApiClient().post<RequestMutation>(`${requestPath(data)}/comments`, {
    auth: 'required',
    body: { body: data.body },
  })
}

export async function markRequestNeedsResponseForRequest(
  data: NeedsResponseInput,
): Promise<RequestMutation> {
  return createApiClient().post<RequestMutation>(
    `${requestPath(data)}/needs-response`,
    {
      auth: 'required',
      body: { body: data.body },
    },
  )
}

export async function respondToRequestForRequest(
  data: RespondRequestInput,
): Promise<RequestMutation> {
  return createApiClient().post<RequestMutation>(`${requestPath(data)}/respond`, {
    auth: 'required',
    body: { body: data.body },
  })
}

export async function resolveRequestForRequest(
  data: ResolveRequestInput,
): Promise<RequestMutation> {
  return createApiClient().post<RequestMutation>(`${requestPath(data)}/resolve`, {
    auth: 'required',
    body: {
      body: data.body,
      disposition: data.disposition,
    },
  })
}

export async function mergeRequestForRequest(
  data: MergeRequestInput,
): Promise<RequestMutation> {
  return createApiClient().post<RequestMutation>(`${requestPath(data)}/merge`, {
    auth: 'required',
    body: {
      body: data.body,
      expected_head_oid: data.expected_head_oid,
      expected_main_oid: data.expected_main_oid,
    },
  })
}

function requestCollectionPath(data: RepoParams) {
  return `/v1/repos/${data.owner}/${data.repo}/requests`
}

function requestPath(data: RequestParams) {
  return `${requestCollectionPath(data)}/${encodeURIComponent(data.request_id)}`
}
