import {
  authHeaders,
  getApiConnection,
  loadJson,
  readRequestAuthToken,
} from '@/api/client'
import { parseRepoParams } from './repo-detail'
import type {
  CommitDetail,
  CommitDetailInput,
  CommitFileDiffInput,
  CommitHistory,
  CommitHistoryInput,
  ProjectionPreviewAudience,
  ReviewFileDiff,
} from './types'

export async function loadCommitHistoryForRequest(
  data: CommitHistoryInput,
): Promise<CommitHistory> {
  const idToken = await readRequestAuthToken()
  const query = new URLSearchParams({
    audience: parseAudience(data.audience),
  })

  return loadJson<CommitHistory>(
    `${getApiConnection()}/v1/repos/${data.owner}/${data.repo}/commits?${query}`,
    { headers: authHeaders(idToken) },
  )
}

export async function loadCommitDetailForRequest(
  data: CommitDetailInput,
): Promise<CommitDetail> {
  const idToken = await readRequestAuthToken()
  const query = new URLSearchParams({
    audience: parseAudience(data.audience),
  })

  return loadJson<CommitDetail>(
    `${getApiConnection()}/v1/repos/${data.owner}/${data.repo}/commits/${encodeURIComponent(data.commit)}?${query}`,
    { headers: authHeaders(idToken) },
  )
}

export async function loadCommitFileDiffForRequest(
  data: CommitFileDiffInput,
): Promise<ReviewFileDiff> {
  const idToken = await readRequestAuthToken()
  const query = new URLSearchParams({
    audience: parseAudience(data.audience),
    path: data.path,
  })

  return loadJson<ReviewFileDiff>(
    `${getApiConnection()}/v1/repos/${data.owner}/${data.repo}/commits/${encodeURIComponent(data.commit)}/file-diff?${query}`,
    { headers: authHeaders(idToken) },
  )
}

export function parseCommitHistoryInput(input: unknown): CommitHistoryInput {
  return {
    ...parseRepoParams(input),
    audience: parseOptionalAudience(input),
  }
}

export function parseCommitDetailInput(input: unknown): CommitDetailInput {
  const data = input as Partial<CommitDetailInput> | null
  const commit = typeof data?.commit === 'string' ? data.commit.trim() : ''
  if (!commit) {
    throw new Error('A commit id is required.')
  }

  return {
    ...parseCommitHistoryInput(input),
    commit,
  }
}

export function parseCommitFileDiffInput(input: unknown): CommitFileDiffInput {
  const data = input as Partial<CommitFileDiffInput> | null
  const path = typeof data?.path === 'string' ? data.path.trim() : ''
  if (!path) {
    throw new Error('A file path is required.')
  }

  return {
    ...parseCommitDetailInput(input),
    path,
  }
}

function parseOptionalAudience(input: unknown): ProjectionPreviewAudience {
  const data = input as Partial<CommitHistoryInput> | null
  return parseAudience(data?.audience)
}

function parseAudience(
  audience: ProjectionPreviewAudience | null | undefined,
): ProjectionPreviewAudience {
  return audience === 'owner' ? 'owner' : 'public'
}
