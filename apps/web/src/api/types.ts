export type Visibility = 'Private' | 'Public'
export type VisibilityState = Visibility | 'Mixed'
export type RepoRole = 'Reader' | 'Writer' | 'Maintainer' | 'Owner'
export type RepoLifecycleState =
  | 'PendingFirstPush'
  | 'PendingPublish'
  | 'Published'
export type RepoPublicationState = RepoLifecycleState
export type TokenStatus = 'Active' | 'Expired' | 'Used'
export type ReviewKind = 'PendingImport' | 'StagedUpdate'
export type StagedFileChangeKind = 'Added' | 'Modified' | 'Deleted'

export type AccountSession = {
  identity: {
    pairwise_sub: string
    email: string | null
    email_verified: boolean
  } | null
  user: {
    id: string
    handle: string
    email: string
    email_verified: boolean
  } | null
}

export type RepoSummary = {
  id: string
  owner_handle: string
  name: string
  lifecycle_state: RepoLifecycleState
  default_visibility: Visibility
  role: RepoRole | null
  staged_update_pending: boolean
}

export type RepoFile = {
  path: string
  oid: string
  tracked: boolean
  visibility: Visibility
}

export type RepoDetail = {
  files: RepoFile[]
  kind: 'repo'
  repo: RepoSummary
}

export type RepoParams = {
  owner: string
  repo: string
}

export type FirstPushToken = {
  status: TokenStatus
  created_at_unix: number
  expires_at_unix: number
  used_at_unix: number | null
  secret: string | null
}

export type GitPushToken = {
  created_at_unix: number
  secret: string | null
}

export type RepoSetup = {
  repo: RepoSummary
  git_remote_path: string
  remote_name: string
  push_branch: string
  push_enabled: boolean
  token: FirstPushToken | null
  push_token: GitPushToken | null
}

export type RepoSetupView = RepoSetup & {
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

export type CreateRepoInput = {
  name: string
  visibility: Visibility
}

export type CreateRepoResponse = {
  repo: RepoSummary
  setup: RepoSetup
}

export type DeleteRepoInput = {
  owner: string
  repo: string
}

export type DeleteRepoResponse = {
  id: string
  deleted: boolean
}

export type PendingImportPayload = {
  publication_state: 'PendingPublish'
  default_visibility: Visibility
  files: RepoFile[]
}

export type StagedFile = {
  path: string
  kind: StagedFileChangeKind
  old_oid: string | null
  new_oid: string | null
  visibility: Visibility
}

export type StagedUpdate = {
  id: string
  branch: string
  base_live_commit_id: string | null
  message: string
  files: StagedFile[]
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
  files: StagedFile[]
}

export type RepoReview = PendingImportReview | StagedUpdateReview
export type RepoReviewResult = RepoReview | { kind: 'NoReview' }
export type ReviewFile = RepoFile | StagedFile

export type SetVisibilityInput = RepoParams & {
  kind: ReviewKind
  paths: string[]
  visibility: Visibility
}
