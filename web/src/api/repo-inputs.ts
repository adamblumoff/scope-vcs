import type {
  CreateRepoInviteInput,
  DeleteRepoMemberInput,
  RepoInviteTokenInput,
  RepoMemberPermissions,
  RepoParams,
  SetRepoFileVisibilityInput,
  UpdateRepoMemberInput,
  UpdateRepoSettingsInput,
} from './types'

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

export function parseCreateRepoInviteInput(
  input: unknown,
): CreateRepoInviteInput {
  const data = input as Partial<CreateRepoInviteInput> | null
  const { owner, repo } = parseRepoParamsInput(data)
  const email = typeof data?.email === 'string' ? data.email.trim() : ''

  if (!owner || !repo) {
    throw new Error('Repository settings route is incomplete.')
  }

  if (!email) {
    throw new Error('Invite email is required.')
  }

  return {
    owner,
    repo,
    email,
    permissions: parseMemberPermissions(data?.permissions),
  }
}

export function parseUpdateRepoMemberInput(
  input: unknown,
): UpdateRepoMemberInput {
  const data = input as Partial<UpdateRepoMemberInput> | null
  const { owner, repo } = parseRepoParamsInput(data)
  const memberUserId =
    typeof data?.member_user_id === 'string'
      ? data.member_user_id.trim()
      : ''

  if (!owner || !repo || !memberUserId) {
    throw new Error('Repository member route is incomplete.')
  }

  return {
    owner,
    repo,
    member_user_id: memberUserId,
    permissions: parseMemberPermissions(data?.permissions),
  }
}

export function parseDeleteRepoMemberInput(
  input: unknown,
): DeleteRepoMemberInput {
  const data = input as Partial<DeleteRepoMemberInput> | null
  const { owner, repo } = parseRepoParamsInput(data)
  const memberUserId =
    typeof data?.member_user_id === 'string'
      ? data.member_user_id.trim()
      : ''

  if (!owner || !repo || !memberUserId) {
    throw new Error('Repository member route is incomplete.')
  }

  return { owner, repo, member_user_id: memberUserId }
}

export function parseRepoInviteTokenInput(
  input: unknown,
): RepoInviteTokenInput {
  const data = input as Partial<RepoInviteTokenInput> | null
  const token = typeof data?.token === 'string' ? data.token.trim() : ''

  if (!token) {
    throw new Error('Invite token is missing.')
  }

  return { token }
}

function parseMemberPermissions(input: unknown): RepoMemberPermissions {
  const data = input as Partial<RepoMemberPermissions> | null
  return {
    can_apply_changes: data?.can_apply_changes === true,
    can_change_file_visibility:
      data?.can_change_file_visibility === true,
    can_push: data?.can_push === true,
  }
}

function parseRepoParamsInput(input: Partial<RepoParams> | null) {
  return {
    owner: typeof input?.owner === 'string' ? input.owner.trim() : '',
    repo: typeof input?.repo === 'string' ? input.repo.trim() : '',
  }
}
