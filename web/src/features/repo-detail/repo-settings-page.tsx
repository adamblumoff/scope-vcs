import type {
  CreateRepoInviteInput,
  CreateRepoInviteResponse,
  DeleteRepoInviteInput,
  DeleteRepoMemberInput,
  DeleteRepoResponse,
  RepoCollaboration,
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
import { useReducer } from 'react'
import { DeleteRepositoryDialog } from './delete-repository-dialog'
import {
  MemberAccessSections,
  RepositoryMembersSection,
} from './repo-members-section'
import { SettingsSections } from './repo-settings-sections'
import { useRepoLayout } from './repo-layout-context'
import {
  initialRepoSettingsPageState,
  repoSettingsPageReducer,
} from './repo-settings-state'

export function RepoSettingsPage({
  createInvite,
  deleteInvite,
  deleteMember,
  deleteRepo,
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
  initialCollaboration: RepoCollaboration | null
  params: RepoParams
  updateMember: (input: UpdateRepoMemberInput) => Promise<RepoMember>
}) {
  const navigate = useNavigate()
  const router = useRouter()
  const { repo } = useRepoLayout()
  const [state, dispatch] = useReducer(
    repoSettingsPageReducer,
    initialRepoSettingsPageState,
  )
  const collaboration = initialCollaboration
  const { deleteError, deleteTarget } = state

  async function mutateAndRefresh<T>(mutation: Promise<T>) {
    const result = await mutation
    await router.invalidate()
    return result
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
    return mutateAndRefresh(createInvite(input))
  }

  async function updateRepositoryMember(input: UpdateRepoMemberInput) {
    return mutateAndRefresh(updateMember(input))
  }

  async function removeRepositoryMember(memberUserId: string) {
    return mutateAndRefresh(
      deleteMember({ ...params, member_user_id: memberUserId }),
    )
  }

  async function removeRepositoryInvite(inviteId: string) {
    return mutateAndRefresh(deleteInvite({ ...params, invite_id: inviteId }))
  }

  return (
    <RepoShell params={params}>
      <PageContent>
        <PageHeader
          badges={(
            <>
              <LifecycleBadge state={repo.lifecycle_state} />
              <Badge variant="neutral">{repo.access.actor}</Badge>
            </>
          )}
          description={(
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
