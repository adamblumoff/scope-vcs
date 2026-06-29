import {
  createApiClient,
  type ApiClient,
  clerkApiTokenTemplate,
  getPublicApiConnection,
} from '@/api/client'
import { loadProjectionPreviewsForRequest } from './projection-preview'
import { loadReviewForRequest } from './review'
import { gitRemoteUrl } from './repo-urls'
import type {
  RepoDetail,
  RepoFile,
  RepoLiveState,
  RepoParams,
  RepoSession,
  RepoSummary,
  SetRepoFileVisibilityInput,
} from './types'
export { parseRepoParams } from './repo-params'

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
      includeOwner: repo.access.actor !== 'Public',
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
    live: repoLiveState(data, repo),
    projection_previews: projectionPreviews,
    repo,
    review,
  } satisfies RepoDetail
}

export async function loadRepoLiveStateForRequest(data: RepoParams) {
  const api = createApiClient()
  const repo = await api.get<RepoSummary>(`/v1/repos/${data.owner}/${data.repo}`, {
    auth: 'optional',
  })
  return repoLiveState(data, repo)
}

function repoLiveState(data: RepoParams, repo: RepoSummary): RepoLiveState {
  return {
    clerk_token_template: clerkApiTokenTemplate(),
    event_stream_url: gitRemoteUrl(
      getPublicApiConnection('building repo event stream URL'),
      `/v1/repos/${encodeURIComponent(data.owner)}/${encodeURIComponent(data.repo)}/events`,
    ),
    repo,
  }
}

async function loadOpenRepoReview(
  data: RepoParams,
  repo: RepoSummary,
  api: ApiClient,
) {
  if (
    !(await api.authenticated()) ||
    (repo.pending_import_pending && repo.access.actor !== 'Owner') ||
    (!repo.pending_import_pending &&
      repo.staged_update_pending &&
      !repo.access.can_apply_changes &&
      !repo.access.can_change_file_visibility &&
      repo.access.actor !== 'Owner') ||
    (!repo.pending_import_pending && !repo.staged_update_pending)
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
