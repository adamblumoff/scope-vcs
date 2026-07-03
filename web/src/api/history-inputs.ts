import { parseRepoParams } from './repo-params'
import type {
  CommitDetailInput,
  CommitFileDiffInput,
  CommitHistoryInput,
  ProjectionPreviewAudience,
} from './types'

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

export function parseCommitHistoryAudience(
  audience: unknown,
): ProjectionPreviewAudience {
  if (audience === undefined || audience === null || audience === '') {
    return 'public'
  }
  if (audience === 'private' || audience === 'public') {
    return audience
  }
  throw new Error(`Unsupported commit history audience: ${String(audience)}`)
}

function parseOptionalAudience(input: unknown): ProjectionPreviewAudience {
  const data = input as Partial<CommitHistoryInput> | null
  return parseCommitHistoryAudience(data?.audience)
}
