import type {
  DeleteRepoResponse,
  RepoDetail,
  RepoGitCredentialView,
  RepoParams,
  RepoSettings,
  RepoSummary,
  UpdateRepoSettingsInput,
} from '@/api/types'
import { homeFlashKey } from '@/api/client'
import { AppHeader } from '@/components/app-header'
import { VisibilityBadge } from '@/components/visibility-badge'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Link, useNavigate, useRouter } from '@tanstack/react-router'
import { AlertCircle, ArrowLeft, CheckCircle2 } from 'lucide-react'
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
      if (typeof window !== 'undefined') {
        window.sessionStorage.setItem(homeFlashKey, `${target.id} deleted.`)
      }
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

      <section className="mx-auto max-w-[980px] px-4 py-7 sm:px-6 lg:py-9">
        <div className="flex flex-col gap-4 border-b border-border pb-6 sm:flex-row sm:items-end sm:justify-between">
          <div className="min-w-0">
            <div className="mb-3 flex flex-wrap items-center gap-2">
              <Badge variant="outline">{repo.lifecycle_state}</Badge>
              <VisibilityBadge visibility={settings.default_new_file_visibility} />
              {repo.staged_update_pending && (
                <Badge variant="outline">Staged update</Badge>
              )}
            </div>
            <h1 className="truncate text-2xl font-semibold leading-8 sm:text-[32px] sm:leading-10">
              Settings
            </h1>
            <p className="mt-2 truncate font-mono text-sm leading-5 text-muted-foreground">
              {repo.id}
            </p>
          </div>
          <Button asChild size="sm" variant="secondary">
            <Link params={params} to="/repos/$owner/$repo">
              <ArrowLeft className="size-3.5" />
              <span>Repo</span>
            </Link>
          </Button>
        </div>

        {settingsError && (
          <Alert className="mt-6" variant="destructive">
            <AlertCircle className="size-4" />
            <AlertTitle>Settings update failed</AlertTitle>
            <AlertDescription>{settingsError}</AlertDescription>
          </Alert>
        )}

        {gitCredentialError && (
          <Alert className="mt-6" variant="destructive">
            <AlertCircle className="size-4" />
            <AlertTitle>Git credential reset failed</AlertTitle>
            <AlertDescription>{gitCredentialError}</AlertDescription>
          </Alert>
        )}

        {deleteError && (
          <Alert className="mt-6" variant="destructive">
            <AlertCircle className="size-4" />
            <AlertTitle>Repository deletion failed</AlertTitle>
            <AlertDescription>{deleteError}</AlertDescription>
          </Alert>
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
      </section>

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
