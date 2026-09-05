import {
  arrayOf,
  createApiClient,
  clerkApiTokenTemplate,
  getPublicApiConnection,
} from '@/api/client'
import { gitRemoteUrl } from './repo-urls'
import type { RepoContent, RepoLiveState, RepoParams, RepoSummary } from './types'
import { ApiRouteTemplates, buildApiPath } from './types.generated'
import { apiValidators } from './validators.generated'
export { parseRepoParams } from './repo-params'

export async function loadRepoContentForRequest(data: RepoParams, signal?: AbortSignal) {
  const api = createApiClient()
  const files = await api.get(
    repoPath(ApiRouteTemplates.repoFiles, data),
    arrayOf(apiValidators.RepoFileResponse),
    { auth: 'optional', signal },
  )

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
  const repo = await api.get(
    repoPath(ApiRouteTemplates.repo, data),
    apiValidators.RepoSummaryResponse,
    { auth: 'optional' },
  )
  return repoLiveState(data, repo)
}

export async function loadRepoFileForRequest(
  data: RepoParams & { path: string },
  signal?: AbortSignal,
) {
  const api = createApiClient()
  return api.get(
    `${repoPath(ApiRouteTemplates.repoFileContent, data)}?path=${encodeURIComponent(data.path)}`,
    apiValidators.RepoFileContentResponse,
    { auth: 'optional', signal },
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
