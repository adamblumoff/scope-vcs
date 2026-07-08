import type {
  AccountSessionResponse,
  BrowserLoginCompleteResponse,
  CliExchangeGrantResponse,
  CliSessionResponse,
  CliSessionsResponse,
  CommitDetailResponse,
  CommitFileDiffRequest,
  CommitFileResponse,
  CommitHistoryRequest,
  CommitHistoryResponse,
  CommitSummaryResponse,
  DeleteRepoResponse as GeneratedDeleteRepoResponse,
  FirstPushTokenResponse,
  FirstPushTokenStatus,
  ProjectionPreviewAudience as GeneratedProjectionPreviewAudience,
  ProjectionPreviewCommitResponse,
  ProjectionPreviewFileResponse,
  ProjectionPreviewResponse,
  ProjectionPreviewSource as GeneratedProjectionPreviewSource,
  ProjectionPreviewSummaryResponse,
  CommentRequestRequest,
  MergeRequestRequest,
  NeedsResponseRequest,
  AcceptRepositoryInviteResponse,
  CreateRepositoryInviteResponse,
  RequestActorRole,
  RequestBaseAudience,
  RequestDetailResponse,
  RequestDisposition,
  RequestEventKind,
  RequestEventResponse,
  RequestListResponse,
  RequestMergeabilityResponse,
  RequestMergeabilityStatus,
  RequestMutationResponse,
  RequestPermissionsResponse,
  RequestSettlementResponse,
  RequestState,
  RequestSummaryResponse,
  ResolveRequestRequest,
  RespondRequestRequest,
  ReviewFileDiffResponse,
  RepoFileResponse,
  RepoPublicationState as GeneratedRepoPublicationState,
  RepoSummaryResponse,
  RepositoryAccessResponse,
  RepositoryActor as GeneratedRepositoryActor,
  RepositoryCollaborationResponse,
  RepositoryInviteLookupResponse,
  RepositoryInviteResponse,
  RepositoryMemberPermissions,
  RepositoryMemberResponse,
  SessionCapabilities,
  SessionIdentity as GeneratedSessionIdentity,
  SessionResponse,
  SessionRepo as GeneratedSessionRepo,
  FileChangeKind as GeneratedFileChangeKind,
  UserResponse,
  Visibility as GeneratedVisibility,
} from './types.generated'

export type Visibility = GeneratedVisibility
export type VisibilityState = Visibility | 'Mixed'
export type RepositoryActor = GeneratedRepositoryActor
export type RepoPublicationState = GeneratedRepoPublicationState
export type RepoLifecycleState = RepoPublicationState
export type TokenStatus = FirstPushTokenStatus
export type FileChangeKind = GeneratedFileChangeKind
export type ProjectionPreviewAudience = GeneratedProjectionPreviewAudience
export type ProjectionPreviewSource = GeneratedProjectionPreviewSource

export type SessionIdentity = GeneratedSessionIdentity
export type User = UserResponse
export type AccountSession = AccountSessionResponse
export type BrowserLoginComplete = BrowserLoginCompleteResponse
export type CliExchangeGrant = CliExchangeGrantResponse
export type CliSession = CliSessionResponse
export type CliSessions = CliSessionsResponse
export type RepoSummary = RepoSummaryResponse
export type RepoAccess = RepositoryAccessResponse
export type RepoMemberPermissions = RepositoryMemberPermissions
export type RepoMember = RepositoryMemberResponse
export type RepoInvite = RepositoryInviteResponse
export type RepoCollaboration = RepositoryCollaborationResponse
export type CreateRepoInviteResponse = CreateRepositoryInviteResponse
export type RepoInviteLookup = RepositoryInviteLookupResponse
export type AcceptRepoInviteResponse = AcceptRepositoryInviteResponse
export type RepoFile = RepoFileResponse
export type RepoCapabilities = SessionCapabilities
export type SessionRepo = GeneratedSessionRepo
export type RepoSession = SessionResponse
export type FirstPushToken = FirstPushTokenResponse
export type DeleteRepoResponse = GeneratedDeleteRepoResponse
export type CommitHistory = CommitHistoryResponse
export type CommitSummary = CommitSummaryResponse
export type CommitDetail = CommitDetailResponse
export type CommitFile = CommitFileResponse
export type ReviewFileDiff = ReviewFileDiffResponse
export type ProjectionPreviewFile = ProjectionPreviewFileResponse
export type ProjectionPreviewCommit = ProjectionPreviewCommitResponse
export type ProjectionPreviewSummary = ProjectionPreviewSummaryResponse
export type ProjectionPreview = ProjectionPreviewResponse
export type RequestList = RequestListResponse
export type RequestDetail = RequestDetailResponse
export type RequestMutation = RequestMutationResponse
export type RequestSummary = RequestSummaryResponse
export type RequestPermissions = RequestPermissionsResponse
export type RequestMergeability = RequestMergeabilityResponse
export type RequestMergeabilityState = RequestMergeabilityStatus
export type RequestSettlement = RequestSettlementResponse
export type RequestEvent = RequestEventResponse
export type RequestWorkflowState = RequestState
export type RequestWorkflowDisposition = RequestDisposition
export type RequestWorkflowEventKind = RequestEventKind
export type RequestWorkflowActorRole = RequestActorRole
export type RequestWorkflowBaseAudience = RequestBaseAudience

export type RepoDetail = {
  capabilities: RepoCapabilities
  clone_remote_url: string
  files: RepoFile[]
  kind: 'repo'
  live: RepoLiveState
  projection_previews: ProjectionPreviews
  repo: RepoSummary
}

export type RepoLiveState = {
  clerk_token_template: string
  event_stream_url: string
  repo: RepoSummary
}

export type RepoParams = {
  owner: string
  repo: string
}

export type HomeState = {
  account: AccountSession | null
  cliInstallCommands: CliInstallCommands
  error: string | null
  repositories: RepoSummary[]
}

export type CliInstallCommands = {
  posix: string
  windows: string
}

export type DeleteRepoInput = {
  owner: string
  repo: string
}

export type CreateRepoInviteInput = RepoParams & {
  email: string
  permissions: RepoMemberPermissions
}

export type UpdateRepoMemberInput = RepoParams & {
  member_user_id: string
  permissions: RepoMemberPermissions
}

export type DeleteRepoMemberInput = RepoParams & {
  member_user_id: string
}

export type DeleteRepoInviteInput = RepoParams & {
  invite_id: string
}

export type RepoInviteTokenInput = {
  token: string
}

export type ReviewFile = RepoFile | CommitFile

export type ProjectionPreviews = {
  source: ProjectionPreviewSource
  private: ProjectionPreview | null
  public: ProjectionPreview | null
}

export type ProjectionPreviewInput = RepoParams & {
  audience: ProjectionPreviewAudience
  source: ProjectionPreviewSource
}

export type CommitHistoryInput = RepoParams & CommitHistoryRequest
export type CommitDetailInput = CommitHistoryInput & {
  commit: string
}
export type CommitFileDiffInput = RepoParams & CommitFileDiffRequest & {
  commit: string
}

export type RequestParams = RepoParams & {
  request_id: string
}

export type CommentRequestInput = RequestParams & CommentRequestRequest
export type NeedsResponseInput = RequestParams & NeedsResponseRequest
export type RespondRequestInput = RequestParams & RespondRequestRequest
export type ResolveRequestInput = RequestParams & ResolveRequestRequest
export type MergeRequestInput = RequestParams & MergeRequestRequest
