import {
  createApiClient,
  clerkApiTokenTemplate,
  getPublicApiConnection,
} from '@/api/client'
import { gitRemoteUrl } from './repo-urls'
import type {
  RepoContent,
  RepoFile,
  RepoFileContent,
  RepoLiveState,
  RepoParams,
  RepoSummary,
} from './types'
import { ApiRouteTemplates, buildApiPath } from './types.generated'
export { parseRepoParams } from './repo-params'

export async function loadRepoContentForRequest(data: RepoParams) {
  const api = createApiClient()
  const files = await api.get<RepoFile[]>(repoPath(ApiRouteTemplates.repoFiles, data), {
    auth: 'optional',
  })

  return {
    clone_remote_url: gitRemoteUrl(
      getPublicApiConnection('building clone command'),
      buildApiPath(ApiRouteTemplates.gitRepo, {
        mode: 'public',
        org: data.owner,
        repo: data.repo,
      }),
    ),
    files,
  } satisfies RepoContent
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
