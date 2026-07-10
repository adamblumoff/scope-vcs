import { createApiClient } from '@/api/client'
import { parseCommitHistoryAudience } from './history-inputs'
import type {
  CommitDetail,
  CommitDetailInput,
  CommitFileDiffInput,
  CommitHistory,
  CommitHistoryInput,
  ReviewFileDiff,
} from './types'
import { ApiRouteTemplates, buildApiPath } from './types.generated'
export {
  parseCommitDetailInput,
  parseCommitFileDiffInput,
  parseCommitHistoryInput,
} from './history-inputs'

export async function loadCommitHistoryForRequest(
  data: CommitHistoryInput,
): Promise<CommitHistory> {
  const query = new URLSearchParams({
    audience: parseCommitHistoryAudience(data.audience),
  })

  return createApiClient().get<CommitHistory>(
    `${buildApiPath(ApiRouteTemplates.repoCommits, {
      owner: data.owner,
      repo: data.repo,
    })}?${query}`,
    { auth: 'optional' },
  )
}

export async function loadCommitDetailForRequest(
  data: CommitDetailInput,
): Promise<CommitDetail> {
  const query = new URLSearchParams({
    audience: parseCommitHistoryAudience(data.audience),
  })

  return createApiClient().get<CommitDetail>(
    `${buildApiPath(ApiRouteTemplates.repoCommit, {
      owner: data.owner,
      repo: data.repo,
      commit_id: data.commit,
    })}?${query}`,
    { auth: 'optional' },
  )
}

export async function loadCommitFileDiffForRequest(
  data: CommitFileDiffInput,
): Promise<ReviewFileDiff> {
  const query = new URLSearchParams({
    audience: parseCommitHistoryAudience(data.audience),
    path: data.path,
  })

  return createApiClient().get<ReviewFileDiff>(
    `${buildApiPath(ApiRouteTemplates.repoCommitFileDiff, {
      owner: data.owner,
      repo: data.repo,
      commit_id: data.commit,
    })}?${query}`,
    { auth: 'optional' },
  )
}
