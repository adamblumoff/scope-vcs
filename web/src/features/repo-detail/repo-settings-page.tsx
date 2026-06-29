import type {
  CreateRepoInviteInput,
  CreateRepoInviteResponse,
  DeleteRepoInviteInput,
  DeleteRepoResponse,
  DeleteRepoMemberInput,
  RepoDetail,
  RepoCollaboration,
  RepoInvite,
  RepoMember,
  RepoParams,
  RepoSettings,
  RepoSummary,
  UpdateRepoMemberInput,
  UpdateRepoSettingsInput,
} from '@/api/types'
import { AppHeader } from '@/components/app-header'
import { RepoBreadcrumb } from '@/components/repo-breadcrumb'
import { PageContent, PageHeader } from '@/components/page-header'
import { LifecycleBadge } from '@/components/lifecycle-badge'
import { PageErrorAlert } from '@/components/page-error-alert'
import { VisibilityBadge } from '@/components/visibility-badge'
import { Badge } from '@/components/ui/badge'
import { storeHomeFlash } from '@/lib/home-flash'
import { Link, useNavigate, useRouter } from '@tanstack/react-router'
import { useReducer, useRef, useState } from 'react'
import { DeleteRepositoryDialog } from './delete-repository-dialog'
import {
  MemberAccessSections,
  RepositoryMembersSection,
} from './repo-members-section'
import { SettingsSections } from './repo-settings-sections'
import {
  type SettingKey,
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
  initialSettings,
  params,
  updateMember,
  updateSettings,
}: {
  createInvite: (
    input: CreateRepoInviteInput,
  ) => Promise<CreateRepoInviteResponse>
  deleteInvite: (input: DeleteRepoInviteInput) => Promise<RepoInvite>
  deleteMember: (input: DeleteRepoMemberInput) => Promise<RepoMember>
  deleteRepo: (params: RepoParams) => Promise<DeleteRepoResponse>
  detail: RepoDetail
  initialCollaboration: RepoCollaboration | null
  initialSettings: RepoSettings | null
  params: RepoParams
  updateMember: (input: UpdateRepoMemberInput) => Promise<RepoMember>
  updateSettings: (settings: UpdateRepoSettingsInput) => Promise<RepoSettings>
}) {
  const navigate = useNavigate()
  const router = useRouter()
  const { repo } = detail
  const [state, dispatch] = useReducer(
    repoSettingsPageReducer,
    initialRepoSettingsPageState,
  )
  const settingsSavePendingRef = useRef(false)
  const [collaborationState, setCollaborationState] = useState(() => ({
    base: initialCollaboration,
    value: initialCollaboration,
  }))
  const collaboration =
    collaborationState.base === initialCollaboration
      ? collaborationState.value
      : initialCollaboration
  const settingsOverride =
    initialSettings &&
    state.settingsOverride?.baseSettings === initialSettings
      ? state.settingsOverride
      : null
  const settings = settingsOverride?.settings ?? initialSettings
  const {
    deleteError,
    deleteTarget,
    pendingSetting,
    settingsError,
  } = state
  const settingsSaving = pendingSetting !== null

  if (collaborationState.base !== initialCollaboration) {
    setCollaborationState({
      base: initialCollaboration,
      value: initialCollaboration,
    })
  }

  async function saveSettings(
    nextSettings: RepoSettings,
    pendingKey: SettingKey,
  ) {
    if (!initialSettings || settingsSavePendingRef.current) {
      return
    }

    settingsSavePendingRef.current = true
    dispatch({ key: pendingKey, type: 'settingsStarted' })
    try {
      const updated = await updateSettings({ ...params, ...nextSettings })
      dispatch({
        baseSettings: initialSettings,
        settings: updated,
        type: 'settingsSucceeded',
      })
      await router.invalidate()
    } catch (error) {
      dispatch({
        message:
          error instanceof Error ? error.message : 'settings update failed',
        type: 'settingsFailed',
      })
    } finally {
      settingsSavePendingRef.current = false
    }
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
    <main className="min-h-screen bg-background text-foreground">
      <AppHeader
        breadcrumb={() => <RepoBreadcrumb params={params} section="settings" />}
      />

      <PageContent>
        <PageHeader
          badges={() => (
            <>
              <LifecycleBadge state={repo.lifecycle_state} />
              {settings ? (
                <VisibilityBadge
                  visibility={settings.default_new_file_visibility}
                />
              ) : (
                <Badge variant="neutral">{repo.access.actor}</Badge>
              )}
              {repo.staged_update_pending && (
                <Badge variant="warning">Staged update</Badge>
              )}
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

        {settingsError && (
          <PageErrorAlert title="Settings update failed">
            {settingsError}
          </PageErrorAlert>
        )}

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

        {settings && (
          <SettingsSections
            onDeleteRepository={() =>
              dispatch({ repo, type: 'deleteTargetChanged' })
            }
            onSaveSettings={(nextSettings, pendingKey) =>
              void saveSettings(nextSettings, pendingKey)
            }
            pendingSetting={pendingSetting}
            settings={settings}
            settingsSaving={settingsSaving}
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
    </main>
  )
}
