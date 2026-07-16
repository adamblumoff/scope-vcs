import type {
  CreateRequestDiscussionReplyRequest,
  CreateRequestDiscussionRequest,
  RequestActivityPageResponse,
  RequestActorSummaryResponse,
  RequestDiscussionChangesResponse,
  RequestDiscussionMutationResponse,
  RequestDiscussionPageResponse,
  RequestDiscussionReadResponse,
  RequestDiscussionReplyMutationResponse,
  RequestDiscussionReplyResponse,
  RequestDiscussionStatus,
  RequestDiscussionSummaryResponse,
} from '@/api/types.generated'

export type RequestActorSummary = RequestActorSummaryResponse
export type { RequestDiscussionStatus }
export type RequestDiscussionSort = 'Newest' | 'Recent'
export type RequestDiscussionFilter = 'All' | 'Open'

export type RequestDiscussionReply = RequestDiscussionReplyResponse
export type RequestDiscussion = RequestDiscussionSummaryResponse
export type RequestDiscussionPage = RequestDiscussionPageResponse
export type RequestDiscussionChanges = RequestDiscussionChangesResponse
export type RequestDiscussionReadState = RequestDiscussionReadResponse
export type RequestDiscussionMutation = RequestDiscussionMutationResponse
export type RequestDiscussionReplyMutation =
  RequestDiscussionReplyMutationResponse
export type RequestActivityPage = RequestActivityPageResponse

export type DiscussionPendingState = 'failed' | 'sending'

export type RequestDiscussionView = RequestDiscussion & {
  expanded?: boolean
  initiallyResolved?: boolean
  pending?: DiscussionPendingState
}

export type RequestDiscussionReplyView = RequestDiscussionReply & {
  pending?: DiscussionPendingState
}

export type CreateRequestDiscussionInput = CreateRequestDiscussionRequest
export type CreateRequestDiscussionReplyInput =
  CreateRequestDiscussionReplyRequest
