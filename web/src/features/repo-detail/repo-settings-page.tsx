import type {
  DeleteRepoResponse,
  RepoDetail,
  RepoGitCredentialView,
  RepoParams,
  RepoSettings,
  RepoSummary,
  UpdateRepoSettingsInput,
} from '@/api/types'
import { AppHeader } from '@/components/app-header'
import { PageContent, PageHeader } from '@/components/page-header'
import { PageErrorAlert } from '@/components/page-error-alert'
import { VisibilityBadge } from '@/components/visibility-badge'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { storeHomeFlash } from '@/lib/home-flash'
import { Link, useNavigate, useRouter } from '@tanstack/react-router'
import { CheckCircle2 } from 'lucide-react'
import { useReducer, useRef } from 'react'
import { DeleteRepositoryDialog } from './delete-repository-dialog'
import { SettingsSections } from './repo-settings-sections'
import {
  type SettingKey,
  initialRepoSettingsPageState,
  repoSettingsPageReducer,
} from './repo-settings-state'

export function RepoSettingsPage({
  deleteRepo,
  detail,
  initialSettings,
  params,
  regenerateGitCredential,
  updateSettings,
}: {
  deleteRepo: (params: RepoParams) => Promise<DeleteRepoResponse>
  detail: RepoDetail
  initialSettings: RepoSettings
  params: RepoParams
  regenerateGitCredential: (params: RepoParams) => Promise<RepoGitCredentialView>
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
  const settingsOverride =
    state.settingsOverride?.baseSettings === initialSettings
      ? state.settingsOverride
      : null
  const settings = settingsOverride?.settings ?? initialSettings
  const {
    deleteError,
    deleteTarget,
    gitCredential,
    gitCredentialError,
    gitCredentialPending,
    pendingSetting,
    settingsError,
  } = state
  const settingsSaving = pendingSetting !== null
  const canResetGitCredential = repo.lifecycle_state === 'Published'

  async function saveSettings(
    nextSettings: RepoSettings,
    pendingKey: SettingKey,
  ) {
    if (settingsSavePendingRef.current) {
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

  async function resetGitCredential() {
    if (!canResetGitCredential) {
      return
    }

    dispatch({ type: 'gitCredentialStarted' })
    try {
      const updated = await regenerateGitCredential(params)
      dispatch({ credential: updated, type: 'gitCredentialSucceeded' })
    } catch (error) {
      dispatch({
        message:
          error instanceof Error ? error.message : 'Git credential reset failed',
        type: 'gitCredentialFailed',
      })
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

  return (
    <main className="min-h-screen bg-background text-foreground">
      <AppHeader subtitle={repo.id} subtitleClassName="font-mono" />

      <PageContent>
        <PageHeader
          badges={() => (
            <>
              <Badge variant="outline">{repo.lifecycle_state}</Badge>
              <VisibilityBadge visibility={settings.default_new_file_visibility} />
              {repo.staged_update_pending && (
                <Badge variant="outline">Staged update</Badge>
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

        {gitCredentialError && (
          <PageErrorAlert title="Git credential reset failed">
            {gitCredentialError}
          </PageErrorAlert>
        )}

        {deleteError && (
          <PageErrorAlert title="Repository deletion failed">
            {deleteError}
          </PageErrorAlert>
        )}

        {gitCredential?.push_token.secret && (
          <Alert className="mt-6">
            <CheckCircle2 className="size-4" />
            <AlertTitle>Git credential reset</AlertTitle>
            <AlertDescription>
              Run the refreshed command below in your local repo.
            </AlertDescription>
          </Alert>
        )}

        <SettingsSections
          canResetGitCredential={canResetGitCredential}
          gitCredential={gitCredential}
          gitCredentialPending={gitCredentialPending}
          onDeleteRepository={() =>
            dispatch({ repo, type: 'deleteTargetChanged' })
          }
          onResetGitCredential={() => void resetGitCredential()}
          onSaveSettings={(nextSettings, pendingKey) =>
            void saveSettings(nextSettings, pendingKey)
          }
          pendingSetting={pendingSetting}
          settings={settings}
          settingsSaving={settingsSaving}
        />
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
