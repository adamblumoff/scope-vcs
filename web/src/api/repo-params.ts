import type { RepoParams } from './types'

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
