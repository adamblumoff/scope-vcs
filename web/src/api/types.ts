import type {
  AccountSessionResponse,
  CreateRepoRequest,
  CreateRepoResponse as GeneratedCreateRepoResponse,
  DeleteRepoResponse as GeneratedDeleteRepoResponse,
  FirstPushTokenResponse,
  FirstPushTokenStatus,
  GitPushTokenResponse,
  PendingImportReviewResponse,
  ProjectionPreviewAudience as GeneratedProjectionPreviewAudience,
  ProjectionPreviewCommitResponse,
  ProjectionPreviewFileResponse,
  ProjectionPreviewResponse,
  ProjectionPreviewSource as GeneratedProjectionPreviewSource,
  ProjectionPreviewSummaryResponse,
  ReviewFileDiffRequest,
  ReviewFileDiffResponse,
  RepoFileResponse,
  RepoGitCredentialResponse,
  RepoPublicationState as GeneratedRepoPublicationState,
  RepoRole as GeneratedRepoRole,
  RepoSettingsResponse,
  RepoSetupResponse,
  RepoSummaryResponse,
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
export type RepoRole = GeneratedRepoRole
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
export type RepoSummary = RepoSummaryResponse
export type RepoFile = RepoFileResponse
export type RepoCapabilities = SessionCapabilities
export type SessionRepo = GeneratedSessionRepo
export type RepoSession = SessionResponse
export type FirstPushToken = FirstPushTokenResponse
export type GitPushToken = GitPushTokenResponse
export type RepoSetup = RepoSetupResponse
export type RepoGitCredential = RepoGitCredentialResponse
export type CreateRepoResponse = GeneratedCreateRepoResponse
export type DeleteRepoResponse = GeneratedDeleteRepoResponse
export type RepoSettings = RepoSettingsResponse
export type PendingImportPayload = PendingImportReviewResponse
export type StagedFile = StagedFileResponse
export type StagedUpdate = StagedUpdateResponse
export type ReviewFileDiff = ReviewFileDiffResponse
export type ProjectionPreviewFile = ProjectionPreviewFileResponse
export type ProjectionPreviewCommit = ProjectionPreviewCommitResponse
export type ProjectionPreviewSummary = ProjectionPreviewSummaryResponse
export type ProjectionPreview = ProjectionPreviewResponse

export type RepoDetail = {
  capabilities: RepoCapabilities
  files: RepoFile[]
  kind: 'repo'
  projection_previews: ProjectionPreviews
  repo: RepoSummary
  review: RepoReview | null
}

export type RepoParams = {
  owner: string
  repo: string
}

export type RepoSetupView = RepoSetup & {
  git_remote_url: string
}

export type RepoGitCredentialView = RepoGitCredential & {
  git_remote_url: string
}

export type SetupRouteState =
  | {
      kind: 'setup'
      setup: RepoSetupView
    }
  | {
      kind: 'review'
    }

export type SetupProgressState = 'waiting' | 'opening-review' | 'published'

export type HomeState = {
  account: AccountSession | null
  error: string | null
  repositories: RepoSummary[]
  signedIn: boolean
}

export type CreateRepoInput = Omit<CreateRepoRequest, 'visibility'> & {
  visibility: Visibility
}

export type DeleteRepoInput = {
  owner: string
  repo: string
}

export type UpdateRepoSettingsInput = RepoParams & UpdateRepoSettingsRequest

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

export type SetVisibilityInput = RepoParams &
  UpdateFileVisibilityRequest & {
    kind: ReviewKind
  }

export type SetRepoFileVisibilityInput = RepoParams & UpdateFileVisibilityRequest
