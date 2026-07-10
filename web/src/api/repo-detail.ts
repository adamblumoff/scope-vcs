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
import { ApiRouteTemplates, buildApiPath } from './types.generated'
export { parseRepoParams } from './repo-params'

export async function loadRepoForRequest(data: RepoParams) {
  const api = createApiClient()
  const [repo, session] = await Promise.all([
    api.get<RepoSummary>(repoPath(ApiRouteTemplates.repo, data), {
      auth: 'optional',
    }),
    api.get<RepoSession>(repoPath(ApiRouteTemplates.repoSession, data), {
      auth: 'optional',
    }),
  ])
  const [files, projectionPreviews] = await Promise.all([
    api.get<RepoFile[]>(repoPath(ApiRouteTemplates.repoFiles, data), {
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
      buildApiPath(ApiRouteTemplates.gitRepo, {
        mode: 'public',
        org: repo.owner_handle,
        repo: repo.name,
      }),
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
  const repo = await api.get<RepoSummary>(repoPath(ApiRouteTemplates.repo, data), {
    auth: 'optional',
  })
  return repoLiveState(data, repo)
}

export async function loadRepoFileForRequest(
  data: RepoParams & { path: string },
) {
  const api = createApiClient()
  return api.get<RepoFileContent>(
    `${repoPath(ApiRouteTemplates.repoFileContent, data)}?path=${encodeURIComponent(data.path)}`,
    { auth: 'optional' },
  )
}

function repoLiveState(data: RepoParams, repo: RepoSummary): RepoLiveState {
  return {
    clerk_token_template: clerkApiTokenTemplate(),
    event_stream_url: gitRemoteUrl(
      getPublicApiConnection('building repo event stream URL'),
      repoPath(ApiRouteTemplates.repoEvents, data),
    ),
    repo,
  }
}

function repoPath(template: string, data: RepoParams) {
  return buildApiPath(template, { owner: data.owner, repo: data.repo })
}
