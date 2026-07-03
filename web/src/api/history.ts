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
    `/v1/repos/${data.owner}/${data.repo}/commits?${query}`,
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
    `/v1/repos/${data.owner}/${data.repo}/commits/${encodeURIComponent(data.commit)}?${query}`,
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
    `/v1/repos/${data.owner}/${data.repo}/commits/${encodeURIComponent(data.commit)}/file-diff?${query}`,
    { auth: 'optional' },
  )
}
