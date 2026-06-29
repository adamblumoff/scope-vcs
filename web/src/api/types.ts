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
  PendingImportReviewResponse,
  ProjectionPreviewAudience as GeneratedProjectionPreviewAudience,
  ProjectionPreviewCommitResponse,
  ProjectionPreviewFileResponse,
  ProjectionPreviewResponse,
  ProjectionPreviewSource as GeneratedProjectionPreviewSource,
  ProjectionPreviewSummaryResponse,
  AcceptRepositoryInviteResponse,
  CreateRepositoryInviteResponse,
  ReviewFileDiffRequest,
  ReviewFileDiffResponse,
  ReviewLineDiffResponse,
  RepoFileResponse,
  RepoPublicationState as GeneratedRepoPublicationState,
  RepoSettingsResponse,
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
  StagedFileChangeKind as GeneratedStagedFileChangeKind,
  StagedFileResponse,
  StagedUpdateResponse,
  UpdateFileVisibilityRequest,
  UpdateRepoSettingsRequest,
  UserResponse,
  Visibility as GeneratedVisibility,
} from './types.generated'

export type Visibility = GeneratedVisibility
export type VisibilityState = Visibility | 'Mixed'
export type RepositoryActor = GeneratedRepositoryActor
export type RepoPublicationState = GeneratedRepoPublicationState
export type RepoLifecycleState = RepoPublicationState
export type TokenStatus = FirstPushTokenStatus
export type ReviewKind = 'PendingImport' | 'StagedUpdate'
export type StagedFileChangeKind = GeneratedStagedFileChangeKind
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
export type RepoSettings = RepoSettingsResponse
export type CommitHistory = CommitHistoryResponse
export type CommitSummary = CommitSummaryResponse
export type CommitDetail = CommitDetailResponse
export type CommitFile = CommitFileResponse
export type PendingImportPayload = PendingImportReviewResponse
export type StagedFile = StagedFileResponse
export type StagedUpdate = StagedUpdateResponse
export type ReviewFileDiff = ReviewFileDiffResponse
export type ReviewLineDiff = ReviewLineDiffResponse
export type ProjectionPreviewFile = ProjectionPreviewFileResponse
export type ProjectionPreviewCommit = ProjectionPreviewCommitResponse
export type ProjectionPreviewSummary = ProjectionPreviewSummaryResponse
export type ProjectionPreview = ProjectionPreviewResponse

export type RepoDetail = {
  capabilities: RepoCapabilities
  clone_remote_url: string
  files: RepoFile[]
  kind: 'repo'
  live: RepoLiveState
  projection_previews: ProjectionPreviews
  repo: RepoSummary
  review: RepoReview | null
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

export type UpdateRepoSettingsInput = RepoParams & UpdateRepoSettingsRequest

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

export type PendingImportReview = PendingImportPayload & {
  kind: 'PendingImport'
}

export type StagedUpdateReview = {
  kind: 'StagedUpdate'
  publication_state: 'Published'
  default_visibility: null
  id: string | null
  branch: string | null
  base_live_commit_id: string | null
  message: string | null
  line_diff: ReviewLineDiff
  files: StagedFile[]
}

export type RepoReview = PendingImportReview | StagedUpdateReview
export type RepoReviewResult = RepoReview | { kind: 'NoReview' }
export type ReviewFile = RepoFile | StagedFile

export type ProjectionPreviews = {
  source: ProjectionPreviewSource
  owner: ProjectionPreview | null
  public: ProjectionPreview | null
}

export type ProjectionPreviewInput = RepoParams & {
  audience: ProjectionPreviewAudience
  source: ProjectionPreviewSource
}

export type ReviewFileDiffInput = RepoParams & ReviewFileDiffRequest
export type CommitHistoryInput = RepoParams & CommitHistoryRequest
export type CommitDetailInput = CommitHistoryInput & {
  commit: string
}
export type CommitFileDiffInput = RepoParams & CommitFileDiffRequest & {
  commit: string
}

export type SetVisibilityInput = RepoParams &
  UpdateFileVisibilityRequest & {
    kind: ReviewKind
  }

export type SetRepoFileVisibilityInput = RepoParams & UpdateFileVisibilityRequest
