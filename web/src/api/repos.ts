import {
  HttpError,
  authHeaders,
  getApiConnection,
  getApiMutationConnection,
  getPublicApiConnection,
  loadJson,
  readRequestAuthToken,
} from '@/api/client'
import { authCookieName } from '@/lib/auth'
import type {
  AccountSession,
  CreateRepoInput,
  CreateRepoResponse,
  DeleteRepoInput,
  DeleteRepoResponse,
  HomeState,
  RepoGitCredential,
  RepoGitCredentialView,
  RepoDetail,
  RepoFile,
  RepoParams,
  RepoSettings,
  RepoSession,
  RepoSummary,
  SetRepoFileVisibilityInput,
  UpdateRepoSettingsInput,
} from './types'
import { loadProjectionPreviewsForRequest } from './projection-preview'
import { loadReviewForRequest } from './review'
import { repoGitCredentialView } from './setup-view'

export {
  parseSetRepoFileVisibilityInput,
  parseUpdateRepoSettingsInput,
} from './repo-inputs'

export async function loadHomeForRequest(): Promise<HomeState> {
  const idToken = await readRequestAuthToken()
  if (!idToken) {
    return {
      account: null,
      error: null,
      repositories: [],
      signedIn: false,
    }
  }

  try {
    const api = getApiConnection()
    const init = { headers: authHeaders(idToken) }
    const [account, repositories] = await Promise.all([
      loadJson<AccountSession>(`${api}/v1/session`, init),
      loadJson<RepoSummary[]>(`${api}/v1/repos`, init),
    ])

    return {
      account,
      error: null,
      repositories,
      signedIn: true,
    }
  } catch (error) {
    if (error instanceof HttpError && error.status === 401) {
      const { deleteCookie } = await import('@tanstack/react-start/server')
      deleteCookie(authCookieName, { path: '/' })
      return {
        account: null,
        error: null,
        repositories: [],
        signedIn: false,
      }
    }

    return {
      account: null,
      error: error instanceof Error ? error.message : 'request failed',
      repositories: [],
      signedIn: true,
    }
  }
}

export async function createRepoForRequest(data: CreateRepoInput) {
  const idToken = await readRequestAuthToken()
  if (!idToken) {
    throw new Error('Sign in to create a repository.')
  }

  const response = await fetch(`${getApiMutationConnection()}/v1/repos`, {
    body: JSON.stringify(data),
    headers: {
      ...authHeaders(idToken),
      'content-type': 'application/json',
    },
    method: 'POST',
  })
  const payload = await response.json().catch(() => null)

  if (!response.ok) {
    throw new Error(payload?.error ?? `request failed: ${response.status}`)
  }

  return payload as CreateRepoResponse
}

export async function deleteRepoForRequest(data: DeleteRepoInput) {
  const idToken = await readRequestAuthToken()
  if (!idToken) {
    throw new Error('Sign in to delete a repository.')
  }

  const response = await fetch(
    `${getApiMutationConnection()}/v1/repos/${data.owner}/${data.repo}`,
    {
      headers: authHeaders(idToken),
      method: 'DELETE',
    },
  )
  const payload = await response.json().catch(() => null)

  if (!response.ok) {
    throw new Error(payload?.error ?? `request failed: ${response.status}`)
  }

  return payload as DeleteRepoResponse
}

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

export async function loadRepoSettingsForRequest(
  data: RepoParams,
): Promise<RepoSettings> {
  const idToken = await readRequestAuthToken()
  if (!idToken) {
    throw new Error('Sign in as the repo owner to load repository settings.')
  }

  return loadJson<RepoSettings>(
    `${getApiConnection()}/v1/repos/${data.owner}/${data.repo}/settings`,
    { headers: authHeaders(idToken) },
  )
}

export async function updateRepoSettingsForRequest(
  data: UpdateRepoSettingsInput,
): Promise<RepoSettings> {
  const idToken = await readRequestAuthToken()
  if (!idToken) {
    throw new Error('Sign in as the repo owner to update repository settings.')
  }

  const response = await fetch(
    `${getApiMutationConnection()}/v1/repos/${data.owner}/${data.repo}/settings`,
    {
      body: JSON.stringify({
        default_new_file_visibility: data.default_new_file_visibility,
        review_pushes_before_applying: data.review_pushes_before_applying,
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

  return payload as RepoSettings
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

export function parseCreateRepoInput(input: unknown): CreateRepoInput {
  const data = input as Partial<CreateRepoInput> | null
  const name = typeof data?.name === 'string' ? data.name.trim() : ''
  const visibility = data?.visibility === 'Public' ? 'Public' : 'Private'

  if (!name) {
    throw new Error('Repository name is required.')
  }

  return { name, visibility }
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
