import {
  createApiClient,
  type ApiClient,
  getPublicApiConnection,
} from '@/api/client'
import { loadProjectionPreviewsForRequest } from './projection-preview'
import { loadReviewForRequest } from './review'
import { gitRemoteUrl, repoCloneCredentialView } from './repo-urls'
import type {
  RepoCloneCredential,
  RepoCloneCredentialView,
  RepoDetail,
  RepoFile,
  RepoParams,
  RepoSession,
  RepoSummary,
  SetRepoFileVisibilityInput,
} from './types'

export async function createCloneCredentialForRequest(
  data: RepoParams,
): Promise<RepoCloneCredentialView> {
  const api = createApiClient()
  const payload = await api.post<RepoCloneCredential>(
    `/v1/repos/${data.owner}/${data.repo}/clone-credential`,
    { auth: 'required' },
  )

  return repoCloneCredentialView(
    getPublicApiConnection('building clone command'),
    payload,
  )
}

export async function loadRepoForRequest(data: RepoParams) {
  const api = createApiClient()
  const [repo, session] = await Promise.all([
    api.get<RepoSummary>(`/v1/repos/${data.owner}/${data.repo}`, {
      auth: 'optional',
    }),
    api.get<RepoSession>(`/v1/repos/${data.owner}/${data.repo}/session`, {
      auth: 'optional',
    }),
  ])
  const review = await loadOpenRepoReview(data, repo, api)
  const [files, projectionPreviews] = await Promise.all([
    review
      ? Promise.resolve([])
      : api.get<RepoFile[]>(`/v1/repos/${data.owner}/${data.repo}/files`, {
          auth: 'optional',
        }),
    loadProjectionPreviewsForRequest(data, review ? 'review' : 'live', {
      api,
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
  api: ApiClient,
) {
  if (
    !(await api.authenticated()) ||
    repo.role !== 'Owner' ||
    (repo.lifecycle_state !== 'PendingPublish' && !repo.staged_update_pending)
  ) {
    return null
  }

  const review = await loadReviewForRequest(data, api)
  return review.kind === 'NoReview' ? null : review
}

export async function setRepoFileVisibilityForRequest(
  data: SetRepoFileVisibilityInput,
) {
  const api = createApiClient()
  return api.patch<RepoFile[]>(
    `/v1/repos/${data.owner}/${data.repo}/files/visibility`,
    {
      auth: 'required',
      body: {
        paths: data.paths,
        visibility: data.visibility,
      },
    },
  )
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
