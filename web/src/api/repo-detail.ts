import {
  authHeaders,
  getApiConnection,
  getApiMutationConnection,
  getPublicApiConnection,
  loadJson,
  readRequestAuthToken,
} from '@/api/client'
import { loadProjectionPreviewsForRequest } from './projection-preview'
import { loadReviewForRequest } from './review'
import {
  gitRemoteUrl,
  repoCloneCredentialView,
  repoGitCredentialView,
} from './setup-view'
import type {
  RepoCloneCredential,
  RepoCloneCredentialView,
  RepoDetail,
  RepoFile,
  RepoGitCredential,
  RepoGitCredentialView,
  RepoParams,
  RepoSession,
  RepoSummary,
  SetRepoFileVisibilityInput,
} from './types'

export async function regenerateGitCredentialForRequest(
  data: RepoParams,
): Promise<RepoGitCredentialView> {
  const idToken = await readRequestAuthToken()
  if (!idToken) {
    throw new Error('Sign in as the repo owner to reset Git credentials.')
  }

  const response = await fetch(
    `${getApiMutationConnection()}/v1/repos/${data.owner}/${data.repo}/git-credential`,
    {
      headers: authHeaders(idToken),
      method: 'POST',
    },
  )
  const payload = await response.json().catch(() => null)

  if (!response.ok) {
    throw new Error(payload?.error ?? `request failed: ${response.status}`)
  }

  return repoGitCredentialView(
    getPublicApiConnection('building Git credential command'),
    payload as RepoGitCredential,
  )
}

export async function createCloneCredentialForRequest(
  data: RepoParams,
): Promise<RepoCloneCredentialView> {
  const idToken = await readRequestAuthToken()
  if (!idToken) {
    throw new Error('Sign in as a repo member to clone with credentials.')
  }

  const response = await fetch(
    `${getApiMutationConnection()}/v1/repos/${data.owner}/${data.repo}/clone-credential`,
    {
      headers: authHeaders(idToken),
      method: 'POST',
    },
  )
  const payload = await response.json().catch(() => null)

  if (!response.ok) {
    throw new Error(payload?.error ?? `request failed: ${response.status}`)
  }

  return repoCloneCredentialView(
    getPublicApiConnection('building clone command'),
    payload as RepoCloneCredential,
  )
}

export async function loadRepoForRequest(data: RepoParams) {
  const idToken = await readRequestAuthToken()
  const api = getApiConnection()
  const init = { headers: authHeaders(idToken) }
  const [repo, session] = await Promise.all([
    loadJson<RepoSummary>(`${api}/v1/repos/${data.owner}/${data.repo}`, init),
    loadJson<RepoSession>(
      `${api}/v1/repos/${data.owner}/${data.repo}/session`,
      init,
    ),
  ])
  const review = await loadOpenRepoReview(data, repo, idToken ?? null)
  const [files, projectionPreviews] = await Promise.all([
    review
      ? Promise.resolve([])
      : loadJson<RepoFile[]>(
          `${api}/v1/repos/${data.owner}/${data.repo}/files`,
          init,
        ),
    loadProjectionPreviewsForRequest(data, review ? 'review' : 'live', {
      includeOwner: repo.role === 'Owner',
    }),
  ])

  return {
    capabilities: session.capabilities,
    clone_remote_url: gitRemoteUrl(
      getPublicApiConnection('building clone command'),
      `/git/${repo.owner_handle}/${repo.name}`,
    ),
    files,
    kind: 'repo',
    projection_previews: projectionPreviews,
    repo,
    review,
  } satisfies RepoDetail
}

async function loadOpenRepoReview(
  data: RepoParams,
  repo: RepoSummary,
  idToken: string | null,
) {
  if (
    !idToken ||
    repo.role !== 'Owner' ||
    (repo.lifecycle_state !== 'PendingPublish' && !repo.staged_update_pending)
  ) {
    return null
  }

  const review = await loadReviewForRequest(data)
  return review.kind === 'NoReview' ? null : review
}

export async function setRepoFileVisibilityForRequest(
  data: SetRepoFileVisibilityInput,
) {
  const idToken = await readRequestAuthToken()
  if (!idToken) {
    throw new Error('Sign in as the repo owner to update file visibility.')
  }

  const response = await fetch(
    `${getApiMutationConnection()}/v1/repos/${data.owner}/${data.repo}/files/visibility`,
    {
      body: JSON.stringify({
        paths: data.paths,
        visibility: data.visibility,
      }),
      headers: {
        ...authHeaders(idToken),
        'content-type': 'application/json',
      },
      method: 'PATCH',
    },
  )
  const payload = await response.json().catch(() => null)

  if (!response.ok) {
    throw new Error(payload?.error ?? `request failed: ${response.status}`)
  }

  return payload as RepoFile[]
}

export function parseRepoParams(
  input: unknown,
  message = 'Repository route is incomplete.',
): RepoParams {
  const data = input as Partial<RepoParams> | null
  const owner = typeof data?.owner === 'string' ? data.owner.trim() : ''
  const repo = typeof data?.repo === 'string' ? data.repo.trim() : ''

  if (!owner || !repo) {
    throw new Error(message)
  }

  return { owner, repo }
}
