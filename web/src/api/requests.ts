import { createApiClient } from '@/api/client'
import type {
  AddRequestEditorInput,
  CommentRequestInput,
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
  RemoveRequestEditorInput,
  ResolveRequestInput,
  RespondRequestInput,
} from './types'

export {
  parseAddRequestEditorInput,
  parseCommentRequestInput,
  parseMergeRequestInput,
  parseNeedsResponseInput,
  parseRequestParams,
  parseRemoveRequestEditorInput,
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
  return createApiClient().get<RequestChanges>(`${requestPath(data)}/changes`, {
    auth: 'optional',
  })
}

export async function loadRequestFileDiffForRequest(
  data: RequestParams & { path: string },
): Promise<ReviewFileDiff> {
  return createApiClient().get<ReviewFileDiff>(
    `${requestPath(data)}/file-diff?path=${encodeURIComponent(data.path)}`,
    { auth: 'optional' },
  )
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

export async function deleteRequestForRequest(
  data: DeleteRequestInput,
): Promise<RequestDelete> {
  return createApiClient().delete<RequestDelete>(requestPath(data), {
    auth: 'required',
  })
}

export async function addRequestEditorForRequest(
  data: AddRequestEditorInput,
): Promise<RequestMutation> {
  return createApiClient().post<RequestMutation>(`${requestPath(data)}/editors`, {
    auth: 'required',
    body: { user_id: data.user_id },
  })
}

export async function removeRequestEditorForRequest(
  data: RemoveRequestEditorInput,
): Promise<RequestMutation> {
  return createApiClient().delete<RequestMutation>(
    `${requestPath(data)}/editors/${encodeURIComponent(data.editor_user_id)}`,
    {
      auth: 'required',
    },
  )
}

function requestCollectionPath(data: RepoParams) {
  return `/v1/repos/${data.owner}/${data.repo}/requests`
}

function requestPath(data: RequestParams) {
  return `${requestCollectionPath(data)}/${encodeURIComponent(data.request_id)}`
}
