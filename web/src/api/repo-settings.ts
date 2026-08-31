import { createApiClient } from '@/api/client'
import type {
  AcceptRepoInviteResponse,
  CreateRepoInviteInput,
  CreateRepoInviteResponse,
  DeleteRepoInviteInput,
  DeleteRepoMemberInput,
  DeleteRepoInput,
  RepoCollaboration,
  RepoInvite,
  RepoInviteLookup,
  RepoInviteTokenInput,
  RepoMember,
  RepoParams,
  UpdateRepoMemberInput,
} from './types'
import { ApiRouteTemplates, buildApiPath } from './types.generated'
import { apiValidators } from './validators.generated'

export async function deleteRepoForRequest(data: DeleteRepoInput) {
  return createApiClient().delete(
    repoRoute(ApiRouteTemplates.repo, data),
    apiValidators.DeleteRepoResponse,
    { auth: 'required' },
  )
}

export async function loadRepoCollaborationForRequest(
  data: RepoParams,
): Promise<RepoCollaboration> {
  return createApiClient().get(
    repoRoute(ApiRouteTemplates.repoMembers, data),
    apiValidators.RepositoryCollaborationResponse,
    { auth: 'required' },
  )
}

export async function createRepoInviteForRequest(
  data: CreateRepoInviteInput,
): Promise<CreateRepoInviteResponse> {
  return createApiClient().post(
    repoRoute(ApiRouteTemplates.repoInvites, data),
    apiValidators.CreateRepositoryInviteResponse,
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
  return createApiClient().patch(
    buildApiPath(ApiRouteTemplates.repoMember, {
      owner: data.owner,
      repo: data.repo,
      member_user_id: data.member_user_id,
    }),
    apiValidators.RepositoryMemberResponse,
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
  return createApiClient().delete(
    buildApiPath(ApiRouteTemplates.repoMember, {
      owner: data.owner,
      repo: data.repo,
      member_user_id: data.member_user_id,
    }),
    apiValidators.RepositoryMemberResponse,
    { auth: 'required' },
  )
}

export async function deleteRepoInviteForRequest(
  data: DeleteRepoInviteInput,
): Promise<RepoInvite> {
  return createApiClient().delete(
    buildApiPath(ApiRouteTemplates.repoInvite, {
      owner: data.owner,
      repo: data.repo,
      invite_id: data.invite_id,
    }),
    apiValidators.RepositoryInviteResponse,
    { auth: 'required' },
  )
}

export async function loadRepoInviteForRequest(
  data: RepoInviteTokenInput,
): Promise<RepoInviteLookup> {
  return createApiClient().get(
    buildApiPath(ApiRouteTemplates.repositoryInvite, { token: data.token }),
    apiValidators.RepositoryInviteLookupResponse,
    { auth: 'optional' },
  )
}

export async function acceptRepoInviteForRequest(
  data: RepoInviteTokenInput,
): Promise<AcceptRepoInviteResponse> {
  return createApiClient().post(
    buildApiPath(ApiRouteTemplates.repositoryInviteAccept, {
      token: data.token,
    }),
    apiValidators.AcceptRepositoryInviteResponse,
    { auth: 'required' },
  )
}

function repoRoute(template: string, data: RepoParams) {
  return buildApiPath(template, { owner: data.owner, repo: data.repo })
}
