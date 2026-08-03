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
  AcceptRepositoryInviteResponse,
  CreateRepositoryInviteResponse,
  RequestActorRole,
  RequestAudience,
  RequestDetailResponse,
  RequestEventKind,
  RequestEventResponse,
  RequestListResponse,
  RequestListItemResponse,
  RequestMergeabilityResponse,
  RequestMergeabilityStatus,
  RequestMutationResponse,
  RequestChangeBlockFilesResponse,
  RequestPermissionsResponse,
  RequestRatingResponse,
  RequestRatingsResponse,
  RequestState,
  RequestSummaryResponse,
  ReviewFileDiffResponse,
  RepoFileResponse,
  RepoFileContentResponse,
  RepoLifecycleState as GeneratedRepoLifecycleState,
  RepoSummaryResponse,
  OwnerProfileResponse,
  RepositoryAccessResponse,
  RepositoryActor as GeneratedRepositoryActor,
  RepositoryCollaborationResponse,
  RepositoryInviteLookupResponse,
  RepositoryInviteResponse,
  RepositoryMemberPermissions,
  RepositoryMemberResponse,
  RepositoryOperationsResponse,
  RepositoryRunDetailResponse,
  RepositoryRunAttemptResponse,
  RepositoryRunLogResponse,
  RepositoryRunStepLogPageResponse,
  RepositoryRunStepResponse,
  RepositoryRunState,
  RepositoryRunSummaryResponse,
  RepositoryRunnerResponse,
  RepositoryRunnerState,
  SessionIdentity as GeneratedSessionIdentity,
  FileChangeKind as GeneratedFileChangeKind,
  UserResponse,
  Visibility as GeneratedVisibility,
} from './types.generated'

export type Visibility = GeneratedVisibility
export type VisibilityState = Visibility | 'Mixed'
export type RepositoryActor = GeneratedRepositoryActor
export type RepoLifecycleState = GeneratedRepoLifecycleState
export type TokenStatus = FirstPushTokenStatus
export type FileChangeKind = GeneratedFileChangeKind
export type ProjectionPreviewAudience = GeneratedProjectionPreviewAudience

export type SessionIdentity = GeneratedSessionIdentity
export type User = UserResponse
export type AccountSession = AccountSessionResponse
export type BrowserLoginComplete = BrowserLoginCompleteResponse
export type CliExchangeGrant = CliExchangeGrantResponse
export type CliSession = CliSessionResponse
export type CliSessions = CliSessionsResponse
export type RepoSummary = RepoSummaryResponse
export type OwnerProfile = OwnerProfileResponse
export type RepoAccess = RepositoryAccessResponse
export type RepoMemberPermissions = RepositoryMemberPermissions
export type RepoMember = RepositoryMemberResponse
export type RepoInvite = RepositoryInviteResponse
export type RepoCollaboration = RepositoryCollaborationResponse
export type CreateRepoInviteResponse = CreateRepositoryInviteResponse
export type RepoInviteLookup = RepositoryInviteLookupResponse
export type AcceptRepoInviteResponse = AcceptRepositoryInviteResponse
export type RepoFile = RepoFileResponse
export type RepoFileContent = RepoFileContentResponse
export type RepoOperations = RepositoryOperationsResponse
export type RepoRun = RepositoryRunSummaryResponse
export type RepoRunState = RepositoryRunState
export type RepoRunDetail = RepositoryRunDetailResponse
export type RepoRunAttempt = RepositoryRunAttemptResponse
export type RepoRunLog = RepositoryRunLogResponse
export type RepoRunStep = RepositoryRunStepResponse
export type RepoRunStepLogPage = RepositoryRunStepLogPageResponse
export type RepoRunner = RepositoryRunnerResponse
export type RepoRunnerState = RepositoryRunnerState
export type FirstPushToken = FirstPushTokenResponse
export type DeleteRepoResponse = GeneratedDeleteRepoResponse
export type CommitHistory = CommitHistoryResponse
export type CommitSummary = CommitSummaryResponse
export type CommitDetail = CommitDetailResponse
export type CommitFile = CommitFileResponse
export type ReviewFileDiff = ReviewFileDiffResponse
export type RequestList = RequestListResponse
export type RequestListItem = RequestListItemResponse
export type RequestDetail = RequestDetailResponse
export type RequestMutation = RequestMutationResponse
export type RequestChangeBlockFiles = RequestChangeBlockFilesResponse
export type RequestSummary = RequestSummaryResponse
export type RequestPermissions = RequestPermissionsResponse
export type RequestRating = RequestRatingResponse
export type RequestRatings = RequestRatingsResponse
export type RequestMergeability = RequestMergeabilityResponse
export type RequestMergeabilityState = RequestMergeabilityStatus
export type RequestEvent = RequestEventResponse
export type RequestWorkflowState = RequestState
export type RequestWorkflowEventKind = RequestEventKind
export type RequestWorkflowActorRole = RequestActorRole
export type RequestWorkflowAudience = RequestAudience

export type RepoContent = {
  clone_remote_url: string
  files: RepoFile[]
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

export type RunActionInput = RepoParams & {
  run_id: string
}

export type RunStepLogsInput = RunActionInput & {
  after: number
  attempt_id: string
  step_index: number
}

export type ProfileState = {
  account: AccountSession
  cliInstallCommands: CliInstallCommands
  profile: OwnerProfile
}

export type CliInstallCommands = {
  posix: string
  windows: string
}

export type CliPlatform = keyof CliInstallCommands

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
