import type {
  CreateRepoInviteInput,
  CreateRepoInviteResponse,
  DeleteRepoInviteInput,
  DeleteRepoMemberInput,
  DeleteRepoResponse,
  RepoCollaboration,
  RepoDetail,
  RepoInvite,
  RepoMember,
  RepoParams,
  RepoSummary,
  UpdateRepoMemberInput,
} from '@/api/types'
import { LifecycleBadge } from '@/components/lifecycle-badge'
import { PageContent, PageHeader } from '@/components/page-header'
import { PageErrorAlert } from '@/components/page-error-alert'
import { RepoShell } from '@/components/repo-shell'
import { Badge } from '@/components/ui/badge'
import { storeHomeFlash } from '@/lib/home-flash'
import { Link, useNavigate, useRouter } from '@tanstack/react-router'
import { useReducer, useState } from 'react'
import { DeleteRepositoryDialog } from './delete-repository-dialog'
import {
  MemberAccessSections,
  RepositoryMembersSection,
} from './repo-members-section'
import { SettingsSections } from './repo-settings-sections'
import {
  initialRepoSettingsPageState,
  repoSettingsPageReducer,
} from './repo-settings-state'

export function RepoSettingsPage({
  createInvite,
  deleteInvite,
  deleteMember,
  deleteRepo,
  detail,
  initialCollaboration,
  params,
  updateMember,
}: {
  createInvite: (
    input: CreateRepoInviteInput,
  ) => Promise<CreateRepoInviteResponse>
  deleteInvite: (input: DeleteRepoInviteInput) => Promise<RepoInvite>
  deleteMember: (input: DeleteRepoMemberInput) => Promise<RepoMember>
  deleteRepo: (params: RepoParams) => Promise<DeleteRepoResponse>
  detail: RepoDetail
  initialCollaboration: RepoCollaboration | null
  params: RepoParams
  updateMember: (input: UpdateRepoMemberInput) => Promise<RepoMember>
}) {
  const navigate = useNavigate()
  const router = useRouter()
  const { repo } = detail
  const [state, dispatch] = useReducer(
    repoSettingsPageReducer,
    initialRepoSettingsPageState,
  )
  const [collaborationState, setCollaborationState] = useState(() => ({
    base: initialCollaboration,
    value: initialCollaboration,
  }))
  const collaboration =
    collaborationState.base === initialCollaboration
      ? collaborationState.value
      : initialCollaboration
  const { deleteError, deleteTarget } = state

  if (collaborationState.base !== initialCollaboration) {
    setCollaborationState({
      base: initialCollaboration,
      value: initialCollaboration,
    })
  }

  async function deleteRepository(target: RepoSummary) {
    dispatch({ repo: target, type: 'deleteStarted' })
    try {
      await deleteRepo({
        owner: target.owner_handle,
        repo: target.name,
      })
      storeHomeFlash(`${target.id} deleted.`)
      await navigate({ to: '/' })
      void router.invalidate().catch(() => undefined)
    } catch (error) {
      dispatch({
        message:
          error instanceof Error ? error.message : 'repository deletion failed',
        type: 'deleteFailed',
      })
      throw error
    }
  }

  async function createMemberInvite(input: CreateRepoInviteInput) {
    const response = await createInvite(input)
    setCollaborationState((current) => ({
      base: current.base,
      value: current.value
        ? {
            ...current.value,
            invites: [
              response.invite,
              ...current.value.invites.filter(
                (invite) => invite.id !== response.invite.id,
              ),
            ],
          }
        : current.value,
    }))
    return response
  }

  async function updateRepositoryMember(input: UpdateRepoMemberInput) {
    const member = await updateMember(input)
    setCollaborationState((current) => ({
      base: current.base,
      value: current.value
        ? {
            ...current.value,
            members: current.value.members.map((candidate) =>
              candidate.user_id === member.user_id ? member : candidate,
            ),
          }
        : current.value,
    }))
    await router.invalidate()
    return member
  }

  async function removeRepositoryMember(memberUserId: string) {
    const member = await deleteMember({
      ...params,
      member_user_id: memberUserId,
    })
    setCollaborationState((current) => ({
      base: current.base,
      value: current.value
        ? {
            ...current.value,
            members: current.value.members.filter(
              (candidate) => candidate.user_id !== member.user_id,
            ),
          }
        : current.value,
    }))
    await router.invalidate()
    return member
  }

  async function removeRepositoryInvite(inviteId: string) {
    const invite = await deleteInvite({
      ...params,
      invite_id: inviteId,
    })
    setCollaborationState((current) => ({
      base: current.base,
      value: current.value
        ? {
            ...current.value,
            invites: current.value.invites.map((candidate) =>
              candidate.id === invite.id ? invite : candidate,
            ),
          }
        : current.value,
    }))
    await router.invalidate()
    return invite
  }

  return (
    <RepoShell
      active="settings"
      canManage={repo.access.actor !== 'Public'}
      params={params}
    >
      <PageContent>
        <PageHeader
          badges={() => (
            <>
              <LifecycleBadge state={repo.lifecycle_state} />
              <Badge variant="neutral">{repo.access.actor}</Badge>
            </>
          )}
          description={() => (
            <Link
              className="font-mono underline underline-offset-4 hover:text-foreground"
              params={{ owner: repo.owner_handle, repo: repo.name }}
              to="/repos/$owner/$repo"
            >
              {repo.id}
            </Link>
          )}
          title="Settings"
        />

        {deleteError && (
          <PageErrorAlert title="Repository deletion failed">
            {deleteError}
          </PageErrorAlert>
        )}

        {repo.access.actor === 'Public' && (
          <PageErrorAlert title="Settings unavailable">
            Sign in as the owner or a repository member to view repository
            access.
          </PageErrorAlert>
        )}

        {repo.access.actor === 'Owner' && (
          <SettingsSections
            onDeleteRepository={() =>
              dispatch({ repo, type: 'deleteTargetChanged' })
            }
          />
        )}

        {repo.access.actor === 'Member' && (
          <MemberAccessSections repo={repo} />
        )}

        {collaboration && (
          <RepositoryMembersSection
            collaboration={collaboration}
            createInvite={createMemberInvite}
            deleteInvite={removeRepositoryInvite}
            deleteMember={removeRepositoryMember}
            params={params}
            repo={repo}
            updateMember={updateRepositoryMember}
          />
        )}
      </PageContent>

      {deleteTarget && (
        <DeleteRepositoryDialog
          onCancel={() =>
            dispatch({ repo: null, type: 'deleteTargetChanged' })
          }
          onConfirm={deleteRepository}
          repo={deleteTarget}
        />
      )}
    </RepoShell>
  )
}
