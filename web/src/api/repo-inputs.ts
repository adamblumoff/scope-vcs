import type { SetRepoFileVisibilityInput } from './types'

export function parseSetRepoFileVisibilityInput(
  input: unknown,
): SetRepoFileVisibilityInput {
  const data = input as Partial<SetRepoFileVisibilityInput> | null
  const owner = typeof data?.owner === 'string' ? data.owner.trim() : ''
  const repo = typeof data?.repo === 'string' ? data.repo.trim() : ''
  const paths = Array.isArray(data?.paths)
    ? data.paths
        .filter((path): path is string => typeof path === 'string')
        .map((path) => path.trim())
        .filter(Boolean)
    : []
  const visibility = data?.visibility === 'Public' ? 'Public' : 'Private'

  if (!owner || !repo) {
    throw new Error('Repository route is incomplete.')
  }

  if (paths.length === 0) {
    throw new Error('At least one file path is required.')
  }

  return { owner, repo, paths, visibility }
}
