import type { RepoParams, SetRepoFileVisibilityInput, UpdateRepoSettingsInput } from './types'

export function parseSetRepoFileVisibilityInput(
  input: unknown,
): SetRepoFileVisibilityInput {
  const data = input as Partial<SetRepoFileVisibilityInput> | null
  const { owner, repo } = parseRepoParamsInput(data)
  const paths = Array.isArray(data?.paths)
    ? data.paths.flatMap((path) => {
        if (typeof path !== 'string') {
          return []
        }

        const trimmed = path.trim()
        return trimmed ? [trimmed] : []
      })
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

export function parseUpdateRepoSettingsInput(
  input: unknown,
): UpdateRepoSettingsInput {
  const data = input as Partial<UpdateRepoSettingsInput> | null
  const { owner, repo } = parseRepoParamsInput(data)
  const defaultNewFileVisibility =
    data?.default_new_file_visibility === 'Public' ? 'Public' : 'Private'

  if (!owner || !repo) {
    throw new Error('Repository settings route is incomplete.')
  }

  return {
    owner,
    repo,
    default_new_file_visibility: defaultNewFileVisibility,
    review_pushes_before_applying:
      data?.review_pushes_before_applying !== false,
  }
}

function parseRepoParamsInput(input: Partial<RepoParams> | null) {
  return {
    owner: typeof input?.owner === 'string' ? input.owner.trim() : '',
    repo: typeof input?.repo === 'string' ? input.repo.trim() : '',
  }
}
