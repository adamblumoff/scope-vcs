import { createApiClient } from '@/api/client'
import type { RequestParams } from '@/api/types'
import { ApiRouteTemplates, buildApiPath } from '@/api/types.generated'
import { apiValidators } from '@/api/validators.generated'
import type {
  CreateRequestDiscussionInput,
  CreateRequestDiscussionReplyInput,
  RequestDiscussionReply,
} from './request-discussion-types'

export type LoadDiscussionsInput = RequestParams & {
  commit_oid?: string
  cursor?: string
  discussion_id?: string
  include_revision_anchor?: boolean
  limit?: number
  revision_id?: string
}

export type LoadRepliesInput = RequestParams & {
  before?: number
  discussion_id: string
  reply?: string
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
  return createApiClient().get(
    `${requestDiscussionsPath(data)}${query({
      commit: data.commit_oid,
      cursor: data.cursor,
      discussion: data.discussion_id,
      include_revision_anchor: data.include_revision_anchor ? 'true' : undefined,
      limit: (data.limit ?? 25).toString(),
      revision: data.revision_id,
    })}`,
    apiValidators.RequestDiscussionPageResponse,
    { auth: 'optional' },
  )
}

export async function loadRequestDiscussionRepliesForRequest(
  data: LoadRepliesInput,
) {
  return createApiClient().get(
    `${requestDiscussionRoute(ApiRouteTemplates.repoRequestDiscussionReplies, data)}${query({
      before: data.before?.toString(),
      limit: '50',
      reply: data.reply,
    })}`,
    apiValidators.RequestDiscussionRepliesPageResponse,
    { auth: 'optional' },
  )
}

export async function loadRequestDiscussionChangesForRequest(
  data: RequestParams & { after: number },
) {
  return createApiClient().get(
    `${requestRoute(ApiRouteTemplates.repoRequestDiscussionChanges, data)}${query({
      after: data.after.toString(),
      limit: '100',
    })}`,
    apiValidators.RequestDiscussionChangesResponse,
    { auth: 'optional' },
  )
}

export async function loadRequestActivityForRequest(
  data: LoadActivityInput,
) {
  return createApiClient().get(
    `${requestRoute(ApiRouteTemplates.repoRequestActivity, data)}${query({
      latest: 'true',
      limit: '50',
    })}`,
    apiValidators.RequestActivityPageResponse,
    { auth: 'optional' },
  )
}

export async function createRequestDiscussionForRequest(
  data: CreateDiscussionInput,
) {
  return createApiClient().post(
    requestDiscussionsPath(data),
    apiValidators.RequestDiscussionMutationResponse,
    {
      auth: 'required',
      body: {
        anchor: data.anchor,
        body_markdown: data.body_markdown,
        client_discussion_id: data.client_discussion_id,
      },
    },
  )
}

export async function createRequestDiscussionReplyForRequest(
  data: CreateReplyInput,
) {
  return createApiClient().post(
    requestDiscussionRoute(
      ApiRouteTemplates.repoRequestDiscussionReplies,
      data,
    ),
    apiValidators.RequestDiscussionReplyMutationResponse,
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
  return createApiClient().post(
    requestDiscussionRoute(
      ApiRouteTemplates.repoRequestDiscussionResolve,
      data,
    ),
    apiValidators.RequestDiscussionMutationResponse,
    { auth: 'required' },
  )
}

export async function reopenAndReplyToRequestDiscussionForRequest(
  data: CreateReplyInput,
) {
  return createApiClient().post(
    requestDiscussionRoute(
      ApiRouteTemplates.repoRequestDiscussionReopenAndReply,
      data,
    ),
    apiValidators.RequestDiscussionReplyMutationResponse,
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
  return createApiClient().put(
    requestDiscussionRoute(
      ApiRouteTemplates.repoRequestDiscussionRead,
      data,
    ),
    apiValidators.RequestDiscussionReadResponse,
    {
      auth: 'required',
      body: { through_position: data.through_position },
    },
  )
}

export async function updateRequestDescriptionForRequest(
  data: UpdateDescriptionInput,
) {
  return createApiClient().patch(
    requestRoute(ApiRouteTemplates.repoRequest, data),
    apiValidators.RequestMutationResponse,
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
