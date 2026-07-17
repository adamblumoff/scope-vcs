import { createApiClient } from '@/api/client'
import type {
  RequestMutation,
  RequestParams,
} from '@/api/types'
import { ApiRouteTemplates, buildApiPath } from '@/api/types.generated'
import type {
  CreateRequestDiscussionInput,
  CreateRequestDiscussionReplyInput,
  RequestActivityPage,
  RequestDiscussionChanges,
  RequestDiscussionMutation,
  RequestDiscussionPage,
  RequestDiscussionReplyMutation,
  RequestDiscussionReply,
  RequestDiscussionReadState,
} from './request-discussion-types'

export type LoadDiscussionsInput = RequestParams & {
  cursor?: string
  sort: 'Newest' | 'Recent'
  status: 'All' | 'Open' | 'Resolved'
}

export type LoadRepliesInput = RequestParams & {
  before?: number
  discussion_id: string
}

export type LoadActivityInput = RequestParams

export type RequestDiscussionActionInput = RequestParams & {
  discussion_id: string
}

export type CreateDiscussionInput = RequestParams & CreateRequestDiscussionInput
export type CreateReplyInput =
  RequestDiscussionActionInput & CreateRequestDiscussionReplyInput
export type MarkDiscussionReadInput = RequestDiscussionActionInput & {
  through_position: number
}
export type UpdateDescriptionInput = RequestParams & {
  description_markdown: string
}

export type RequestDiscussionRepliesPage = {
  next_before_position: number | null
  replies: RequestDiscussionReply[]
}

export async function loadRequestDiscussionsForRequest(
  data: LoadDiscussionsInput,
) {
  return createApiClient().get<RequestDiscussionPage>(
    `${requestDiscussionsPath(data)}${query({
      cursor: data.cursor,
      limit: '25',
      sort: data.sort.toLowerCase(),
      status: data.status.toLowerCase(),
    })}`,
    { auth: 'optional' },
  )
}

export async function loadRequestDiscussionRepliesForRequest(
  data: LoadRepliesInput,
) {
  return createApiClient().get<RequestDiscussionRepliesPage>(
    `${requestDiscussionRoute(ApiRouteTemplates.repoRequestDiscussionReplies, data)}${query({
      before: data.before?.toString(),
      limit: '50',
    })}`,
    { auth: 'optional' },
  )
}

export async function loadRequestDiscussionChangesForRequest(
  data: RequestParams & { after: number },
) {
  return createApiClient().get<RequestDiscussionChanges>(
    `${requestRoute(ApiRouteTemplates.repoRequestDiscussionChanges, data)}${query({
      after: data.after.toString(),
      limit: '100',
    })}`,
    { auth: 'optional' },
  )
}

export async function loadRequestActivityForRequest(
  data: LoadActivityInput,
) {
  return createApiClient().get<RequestActivityPage>(
    `${requestRoute(ApiRouteTemplates.repoRequestActivity, data)}${query({
      latest: 'true',
      limit: '50',
    })}`,
    { auth: 'optional' },
  )
}

export async function createRequestDiscussionForRequest(
  data: CreateDiscussionInput,
) {
  return createApiClient().post<RequestDiscussionMutation>(
    requestDiscussionsPath(data),
    {
      auth: 'required',
      body: {
        body_markdown: data.body_markdown,
        client_discussion_id: data.client_discussion_id,
      },
    },
  )
}

export async function createRequestDiscussionReplyForRequest(
  data: CreateReplyInput,
) {
  return createApiClient().post<RequestDiscussionReplyMutation>(
    requestDiscussionRoute(
      ApiRouteTemplates.repoRequestDiscussionReplies,
      data,
    ),
    {
      auth: 'required',
      body: {
        body_markdown: data.body_markdown,
        client_reply_id: data.client_reply_id,
        reply_to_reply_id: data.reply_to_reply_id,
      },
    },
  )
}

export async function resolveRequestDiscussionForRequest(
  data: RequestDiscussionActionInput,
) {
  return createApiClient().post<RequestDiscussionMutation>(
    requestDiscussionRoute(
      ApiRouteTemplates.repoRequestDiscussionResolve,
      data,
    ),
    { auth: 'required' },
  )
}

export async function reopenRequestDiscussionForRequest(
  data: RequestDiscussionActionInput,
) {
  return createApiClient().post<RequestDiscussionMutation>(
    requestDiscussionRoute(
      ApiRouteTemplates.repoRequestDiscussionReopen,
      data,
    ),
    { auth: 'required' },
  )
}

export async function reopenAndReplyToRequestDiscussionForRequest(
  data: CreateReplyInput,
) {
  return createApiClient().post<RequestDiscussionReplyMutation>(
    requestDiscussionRoute(
      ApiRouteTemplates.repoRequestDiscussionReopenAndReply,
      data,
    ),
    {
      auth: 'required',
      body: {
        body_markdown: data.body_markdown,
        client_reply_id: data.client_reply_id,
        reply_to_reply_id: data.reply_to_reply_id,
      },
    },
  )
}

export async function markRequestDiscussionReadForRequest(
  data: MarkDiscussionReadInput,
) {
  return createApiClient().put<RequestDiscussionReadState>(
    requestDiscussionRoute(
      ApiRouteTemplates.repoRequestDiscussionRead,
      data,
    ),
    {
      auth: 'required',
      body: { through_position: data.through_position },
    },
  )
}

export async function updateRequestDescriptionForRequest(
  data: UpdateDescriptionInput,
) {
  return createApiClient().patch<RequestMutation>(
    requestRoute(ApiRouteTemplates.repoRequestDescription, data),
    {
      auth: 'required',
      body: { description_markdown: data.description_markdown },
    },
  )
}

function requestDiscussionsPath(data: RequestParams) {
  return requestRoute(ApiRouteTemplates.repoRequestDiscussions, data)
}

function requestRoute(template: string, data: RequestParams) {
  return buildApiPath(template, {
    owner: data.owner,
    repo: data.repo,
    request_id: data.request_id,
  })
}

function requestDiscussionRoute(
  template: string,
  data: RequestDiscussionActionInput,
) {
  return buildApiPath(template, {
    discussion_id: data.discussion_id,
    owner: data.owner,
    repo: data.repo,
    request_id: data.request_id,
  })
}

function query(values: Record<string, string | undefined>) {
  const params = new URLSearchParams()
  for (const [key, value] of Object.entries(values)) {
    if (value) params.set(key, value)
  }
  const encoded = params.toString()
  return encoded ? `?${encoded}` : ''
}
