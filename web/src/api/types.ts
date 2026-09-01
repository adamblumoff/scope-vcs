import type {
  AccountSessionResponse,
  BrowserLoginCompleteResponse,
  CliExchangeGrantResponse,
  CliSessionResponse,
  CliSessionsResponse,
  CommitFileResponse,
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
  RequestRevisionListResponse,
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
  RepositoryRunHistoryPageResponse,
  RepositoryRunWorkflowListResponse,
  RepositoryRunWorkflowResponse,
  RepositoryRunDetailResponse,
  RepositoryRunAttemptResponse,
  RepositoryRunCacheResponse,
  RepositoryRunJobDetailResponse,
  RepositoryRunJobResponse,
  RepositoryRunJobState,
  RepositoryRunLogResponse,
  RepositoryRunStepLogPageResponse,
  RepositoryRunStepResponse,
  RepositoryRunState,
  RepositoryRunSummaryResponse,
  RepositoryRunTerminalReason,
  RepositoryRunTrigger,
  SessionIdentity as GeneratedSessionIdentity,
  FileChangeKind as GeneratedFileChangeKind,
  HistoryEntryDetailResponse,
  HistoryEntryFileDiffRequest,
  HistoryEntryRequest,
  HistoryEntryKind as GeneratedHistoryEntryKind,
  HistoryEntrySummaryResponse,
  HistoryPageRequest,
  HistoryPageResponse,
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
export type HistoryEntryKind = GeneratedHistoryEntryKind

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
export type RepoRun = RepositoryRunSummaryResponse
export type RepoRunHistoryPage = RepositoryRunHistoryPageResponse
export type RepoRunWorkflowList = RepositoryRunWorkflowListResponse
export type RepoRunWorkflow = RepositoryRunWorkflowResponse
export type RepoRunState = RepositoryRunState
export type RepoRunDetail = RepositoryRunDetailResponse
export type RepoRunAttempt = RepositoryRunAttemptResponse
export type RepoRunCache = RepositoryRunCacheResponse
export type RepoRunJobDetail = RepositoryRunJobDetailResponse
export type RepoRunJob = RepositoryRunJobResponse
export type RepoRunJobState = RepositoryRunJobState
export type RepoRunLog = RepositoryRunLogResponse
export type RepoRunStep = RepositoryRunStepResponse
export type RepoRunTerminalReason = RepositoryRunTerminalReason
export type RepoRunTrigger = RepositoryRunTrigger
export type RepoRunStepLogPage = RepositoryRunStepLogPageResponse
export type FirstPushToken = FirstPushTokenResponse
export type DeleteRepoResponse = GeneratedDeleteRepoResponse
export type CommitFile = CommitFileResponse
export type CommitSummary = {
  projected_id: string
  logical_commit_id: string
  parent_projected_id: string | null
  author: string | null
  message: string
  change_count: number
}
export type CommitDetail = CommitSummary & {
  audience: ProjectionPreviewAudience
  files_truncated: boolean
  repo_id: string
  view_key: string
  files: CommitFile[]
}
export type HistoryPage = HistoryPageResponse
export type HistoryEntrySummary = HistoryEntrySummaryResponse
export type HistoryEntryDetail = HistoryEntryDetailResponse
export type ReviewDiffBinarySide = {
  label: 'New' | 'Old'
  oid: string
  sizeBytes: number
}

export type ReviewDiffTextSide = {
  content: string
  label: 'New' | 'Old'
  truncated: boolean
}

export type ReviewDiffOmittedReason = 'hunks' | 'input' | 'lines' | 'output'

export type ReviewDiffPresentation =
  | { kind: 'binary'; sides: ReviewDiffBinarySide[] }
  | { kind: 'empty' }
  | { html: string; kind: 'html' }
  | {
      binary: ReviewDiffBinarySide[]
      kind: 'mixed'
      text: ReviewDiffTextSide[]
    }
  | { kind: 'omitted'; reason: ReviewDiffOmittedReason }

export type ReviewFileDiff = Pick<
  ReviewFileDiffResponse,
  'kind' | 'new_mode' | 'old_mode' | 'path'
> & {
  presentation: ReviewDiffPresentation
}
export type RequestList = RequestListResponse
export type RequestListItem = RequestListItemResponse
export type RequestDetail = RequestDetailResponse
export type RequestMutation = RequestMutationResponse
export type RequestRevisions = RequestRevisionListResponse
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

export type RepoRunHistoryInput = RepoParams & {
  after?: string
  limit?: number
  workflow?: string
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

export type HistoryPageInput = RepoParams & HistoryPageRequest
export type HistoryEntryDetailInput = RepoParams & HistoryEntryRequest & {
  entry: string
}
export type HistoryEntryFileDiffInput = RepoParams & HistoryEntryFileDiffRequest & {
  entry: string
}

export type RequestParams = RepoParams & {
  request_id: string
}
