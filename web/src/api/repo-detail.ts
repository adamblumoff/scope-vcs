import {
  createApiClient,
  clerkApiTokenTemplate,
  getPublicApiConnection,
} from '@/api/client'
import { loadProjectionPreviewsForRequest } from './projection-preview'
import { gitRemoteUrl } from './repo-urls'
import type {
  RepoDetail,
  RepoFile,
  RepoFileContent,
  RepoLiveState,
  RepoParams,
  RepoSession,
  RepoSummary,
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
  const [files, projectionPreviews] = await Promise.all([
    api.get<RepoFile[]>(`/v1/repos/${data.owner}/${data.repo}/files`, {
      auth: 'optional',
    }),
    loadProjectionPreviewsForRequest(data, 'live', {
      api,
      includePrivate: repo.access.actor !== 'Public',
    }),
  ])

  return {
    capabilities: session.capabilities,
    clone_remote_url: gitRemoteUrl(
      getPublicApiConnection('building clone command'),
      `/git/public/${repo.owner_handle}/${repo.name}`,
    ),
    files,
    kind: 'repo',
    live: repoLiveState(data, repo),
    projection_previews: projectionPreviews,
    repo,
  } satisfies RepoDetail
}

export async function loadRepoLiveStateForRequest(data: RepoParams) {
  const api = createApiClient()
  const repo = await api.get<RepoSummary>(`/v1/repos/${data.owner}/${data.repo}`, {
    auth: 'optional',
  })
  return repoLiveState(data, repo)
}

export async function loadRepoFileForRequest(
  data: RepoParams & { path: string },
) {
  const api = createApiClient()
  return api.get<RepoFileContent>(
    `/v1/repos/${encodeURIComponent(data.owner)}/${encodeURIComponent(data.repo)}/files/content?path=${encodeURIComponent(data.path)}`,
    { auth: 'optional' },
  )
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
