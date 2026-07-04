import { createApiClient } from '@/api/client'
import type {
  AcceptRepoInviteResponse,
  CreateRepoInviteInput,
  CreateRepoInviteResponse,
  DeleteRepoInviteInput,
  DeleteRepoMemberInput,
  DeleteRepoInput,
  DeleteRepoResponse,
  RepoCollaboration,
  RepoInvite,
  RepoInviteLookup,
  RepoInviteTokenInput,
  RepoMember,
  RepoParams,
  UpdateRepoMemberInput,
} from './types'

export async function deleteRepoForRequest(data: DeleteRepoInput) {
  return createApiClient().delete<DeleteRepoResponse>(
    `/v1/repos/${data.owner}/${data.repo}`,
    { auth: 'required' },
  )
}

export async function loadRepoCollaborationForRequest(
  data: RepoParams,
): Promise<RepoCollaboration> {
  return createApiClient().get<RepoCollaboration>(
    `/v1/repos/${data.owner}/${data.repo}/members`,
    { auth: 'required' },
  )
}

export async function createRepoInviteForRequest(
  data: CreateRepoInviteInput,
): Promise<CreateRepoInviteResponse> {
  return createApiClient().post<CreateRepoInviteResponse>(
    `/v1/repos/${data.owner}/${data.repo}/invites`,
    {
      auth: 'required',
      body: {
        email: data.email,
        permissions: data.permissions,
      },
    },
  )
}

export async function updateRepoMemberForRequest(
  data: UpdateRepoMemberInput,
): Promise<RepoMember> {
  return createApiClient().patch<RepoMember>(
    `/v1/repos/${data.owner}/${data.repo}/members/${data.member_user_id}`,
    {
      auth: 'required',
      body: {
        permissions: data.permissions,
      },
    },
  )
}

export async function deleteRepoMemberForRequest(
  data: DeleteRepoMemberInput,
): Promise<RepoMember> {
  return createApiClient().delete<RepoMember>(
    `/v1/repos/${data.owner}/${data.repo}/members/${data.member_user_id}`,
    { auth: 'required' },
  )
}

export async function deleteRepoInviteForRequest(
  data: DeleteRepoInviteInput,
): Promise<RepoInvite> {
  return createApiClient().delete<RepoInvite>(
    `/v1/repos/${data.owner}/${data.repo}/invites/${data.invite_id}`,
    { auth: 'required' },
  )
}

export async function loadRepoInviteForRequest(
  data: RepoInviteTokenInput,
): Promise<RepoInviteLookup> {
  return createApiClient().get<RepoInviteLookup>(
    `/v1/repository-invites/${data.token}`,
    { auth: 'optional' },
  )
}

export async function acceptRepoInviteForRequest(
  data: RepoInviteTokenInput,
): Promise<AcceptRepoInviteResponse> {
  return createApiClient().post<AcceptRepoInviteResponse>(
    `/v1/repository-invites/${data.token}/accept`,
    { auth: 'required' },
  )
}
