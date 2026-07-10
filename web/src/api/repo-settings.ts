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
import { ApiRouteTemplates, buildApiPath } from './types.generated'

export async function deleteRepoForRequest(data: DeleteRepoInput) {
  return createApiClient().delete<DeleteRepoResponse>(
    repoRoute(ApiRouteTemplates.repo, data),
    { auth: 'required' },
  )
}

export async function loadRepoCollaborationForRequest(
  data: RepoParams,
): Promise<RepoCollaboration> {
  return createApiClient().get<RepoCollaboration>(
    repoRoute(ApiRouteTemplates.repoMembers, data),
    { auth: 'required' },
  )
}

export async function createRepoInviteForRequest(
  data: CreateRepoInviteInput,
): Promise<CreateRepoInviteResponse> {
  return createApiClient().post<CreateRepoInviteResponse>(
    repoRoute(ApiRouteTemplates.repoInvites, data),
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
    buildApiPath(ApiRouteTemplates.repoMember, {
      owner: data.owner,
      repo: data.repo,
      member_user_id: data.member_user_id,
    }),
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
    buildApiPath(ApiRouteTemplates.repoMember, {
      owner: data.owner,
      repo: data.repo,
      member_user_id: data.member_user_id,
    }),
    { auth: 'required' },
  )
}

export async function deleteRepoInviteForRequest(
  data: DeleteRepoInviteInput,
): Promise<RepoInvite> {
  return createApiClient().delete<RepoInvite>(
    buildApiPath(ApiRouteTemplates.repoInvite, {
      owner: data.owner,
      repo: data.repo,
      invite_id: data.invite_id,
    }),
    { auth: 'required' },
  )
}

export async function loadRepoInviteForRequest(
  data: RepoInviteTokenInput,
): Promise<RepoInviteLookup> {
  return createApiClient().get<RepoInviteLookup>(
    buildApiPath(ApiRouteTemplates.repositoryInvite, { token: data.token }),
    { auth: 'optional' },
  )
}

export async function acceptRepoInviteForRequest(
  data: RepoInviteTokenInput,
): Promise<AcceptRepoInviteResponse> {
  return createApiClient().post<AcceptRepoInviteResponse>(
    buildApiPath(ApiRouteTemplates.repositoryInviteAccept, {
      token: data.token,
    }),
    { auth: 'required' },
  )
}

function repoRoute(template: string, data: RepoParams) {
  return buildApiPath(template, { owner: data.owner, repo: data.repo })
}
