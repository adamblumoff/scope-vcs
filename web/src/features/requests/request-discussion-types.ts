import type {
  CreateRequestDiscussionReplyRequest,
  CreateRequestDiscussionRequest,
  RequestActivityPageResponse,
  RequestActorSummaryResponse,
  RequestChangeBlockFilesResponse,
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

export type RequestDiscussionReply = RequestDiscussionReplyResponse
export type RequestDiscussion = Omit<RequestDiscussionSummaryResponse, 'change_block'> & {
  change_block?: RequestDiscussionSummaryResponse['change_block']
}
export type RequestDiscussionPage = Omit<RequestDiscussionPageResponse, 'discussions'> & {
  discussions: RequestDiscussion[]
}
export type RequestDiscussionChanges = Omit<RequestDiscussionChangesResponse, 'discussions'> & {
  discussions: RequestDiscussion[]
}
export type RequestDiscussionReadState = RequestDiscussionReadResponse
export type RequestDiscussionMutation = Omit<RequestDiscussionMutationResponse, 'discussion'> & {
  discussion: RequestDiscussion
}
export type RequestDiscussionReplyMutation =
  Omit<RequestDiscussionReplyMutationResponse, 'discussion'> & {
    discussion: RequestDiscussion
  }
export type RequestActivityPage = RequestActivityPageResponse
export type { RequestChangeBlockFilesResponse }

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
