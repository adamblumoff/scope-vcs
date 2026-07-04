import type {
  CreateRepoInviteInput,
  DeleteRepoInviteInput,
  DeleteRepoMemberInput,
  RepoInviteTokenInput,
  RepoMemberPermissions,
  RepoParams,
  UpdateRepoMemberInput,
} from './types'

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

export function parseDeleteRepoInviteInput(
  input: unknown,
): DeleteRepoInviteInput {
  const data = input as Partial<DeleteRepoInviteInput> | null
  const { owner, repo } = parseRepoParamsInput(data)
  const inviteId =
    typeof data?.invite_id === 'string' ? data.invite_id.trim() : ''

  if (!owner || !repo || !inviteId) {
    throw new Error('Repository invite route is incomplete.')
  }

  return { owner, repo, invite_id: inviteId }
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
    can_apply_changes: false,
    can_change_file_visibility: false,
    can_push: data?.can_push === true,
  }
}

function parseRepoParamsInput(input: Partial<RepoParams> | null) {
  return {
    owner: typeof input?.owner === 'string' ? input.owner.trim() : '',
    repo: typeof input?.repo === 'string' ? input.repo.trim() : '',
  }
}
