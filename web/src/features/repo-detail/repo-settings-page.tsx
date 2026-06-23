import type {
  DeleteRepoResponse,
  RepoDetail,
  RepoGitCredentialView,
  RepoParams,
  RepoSettings,
  RepoSummary,
  UpdateRepoSettingsInput,
  Visibility,
} from '@/api/types'
import { homeFlashKey } from '@/api/client'
import { AppHeader } from '@/components/app-header'
import { CopyableCodeBlock } from '@/components/copyable-code-block'
import { VisibilityBadge } from '@/components/visibility-badge'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Link, useNavigate, useRouter } from '@tanstack/react-router'
import {
  AlertCircle,
  ArrowLeft,
  CheckCircle2,
  FilePlus2,
  GitBranch,
  Globe2,
  KeyRound,
  LoaderCircle,
  Lock,
  RefreshCw,
  Trash2,
  Users,
} from 'lucide-react'
import type { ReactNode } from 'react'
import { useReducer, useRef } from 'react'
import { gitCredentialApproveCommand } from '../setup/commands'
import { DeleteRepositoryDialog } from './delete-repository-dialog'

type SettingKey = 'default-new-file-visibility' | 'push-review'

type SettingsOverride = {
  baseSettings: RepoSettings
  settings: RepoSettings
}

type RepoSettingsPageState = {
  deleteError: string | null
  deleteTarget: RepoSummary | null
  gitCredential: RepoGitCredentialView | null
  gitCredentialError: string | null
  gitCredentialPending: boolean
  pendingSetting: SettingKey | null
  settingsError: string | null
  settingsOverride: SettingsOverride | null
}

type RepoSettingsPageAction =
  | { type: 'deleteFailed'; message: string }
  | { type: 'deleteStarted'; repo: RepoSummary }
  | { type: 'deleteTargetChanged'; repo: RepoSummary | null }
  | { type: 'gitCredentialStarted' }
  | { credential: RepoGitCredentialView; type: 'gitCredentialSucceeded' }
  | { message: string; type: 'gitCredentialFailed' }
  | { key: SettingKey; type: 'settingsStarted' }
  | {
      baseSettings: RepoSettings
      settings: RepoSettings
      type: 'settingsSucceeded'
    }
  | { message: string; type: 'settingsFailed' }

const initialRepoSettingsPageState: RepoSettingsPageState = {
  deleteError: null,
  deleteTarget: null,
  gitCredential: null,
  gitCredentialError: null,
  gitCredentialPending: false,
  pendingSetting: null,
  settingsError: null,
  settingsOverride: null,
}

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

function SettingsSections({
  canResetGitCredential,
  gitCredential,
  gitCredentialPending,
  onDeleteRepository,
  onResetGitCredential,
  onSaveSettings,
  pendingSetting,
  settings,
  settingsSaving,
}: {
  canResetGitCredential: boolean
  gitCredential: RepoGitCredentialView | null
  gitCredentialPending: boolean
  onDeleteRepository: () => void
  onResetGitCredential: () => void
  onSaveSettings: (settings: RepoSettings, pendingKey: SettingKey) => void
  pendingSetting: SettingKey | null
  settings: RepoSettings
  settingsSaving: boolean
}) {
  return (
    <div className="mt-8 divide-y divide-border border-y border-border">
      <SettingsRow
        description="Future Git pushes either stop in review or apply directly to the live repo."
        icon={<GitBranch className="size-4" />}
        title="Push workflow"
      >
        <label className="flex items-center gap-3 text-sm leading-5">
          <input
            checked={settings.review_pushes_before_applying}
            className="size-4 accent-primary"
            disabled={settingsSaving}
            onChange={() =>
              onSaveSettings(
                {
                  ...settings,
                  review_pushes_before_applying:
                    !settings.review_pushes_before_applying,
                },
                'push-review',
              )
            }
            type="checkbox"
          />
          <span>Review pushes before applying</span>
          {pendingSetting === 'push-review' && (
            <LoaderCircle className="size-3.5 animate-spin text-muted-foreground" />
          )}
        </label>
      </SettingsRow>

      <SettingsRow
        description="New paths inherit this visibility unless you set a more specific file or folder rule."
        icon={<FilePlus2 className="size-4" />}
        title="Default new file visibility"
      >
        <VisibilityChoice
          current={settings.default_new_file_visibility}
          disabled={settingsSaving}
          onSelect={(visibility) =>
            onSaveSettings(
              {
                ...settings,
                default_new_file_visibility: visibility,
              },
              'default-new-file-visibility',
            )
          }
        />
      </SettingsRow>

      <SettingsRow
        description="Refresh the credential your local Git client uses when pushing to the Scope remote."
        icon={<KeyRound className="size-4" />}
        title="Git credentials"
      >
        <div className="min-w-0 space-y-3">
          <Button
            disabled={!canResetGitCredential || gitCredentialPending}
            onClick={onResetGitCredential}
            size="sm"
            type="button"
            variant="secondary"
          >
            {gitCredentialPending ? (
              <LoaderCircle className="size-3.5 animate-spin" />
            ) : (
              <RefreshCw className="size-3.5" />
            )}
            <span>
              {gitCredentialPending ? 'Resetting' : 'Reset Git credential'}
            </span>
          </Button>
          {!canResetGitCredential && (
            <p className="text-sm leading-5 text-muted-foreground">
              Git credential reset is available after the repo is published.
            </p>
          )}
          {gitCredential?.push_token.secret && (
            <CopyableCodeBlock
              value={gitCredentialApproveCommand(
                gitCredential,
                gitCredential.push_token.secret,
              )}
            />
          )}
        </div>
      </SettingsRow>

      <SettingsRow
        description="Roles are already enforced internally, but member list and invite endpoints are not implemented yet."
        icon={<Users className="size-4" />}
        title="Members"
      >
        <label className="flex items-center gap-3 text-sm leading-5 text-muted-foreground">
          <input className="size-4" disabled type="checkbox" />
          <span>Member management</span>
          <Badge variant="outline">Blocked by API</Badge>
        </label>
      </SettingsRow>

      <SettingsRow
        description="Permanently removes repo metadata, pending review state, and stored Git data from Scope."
        icon={<Trash2 className="size-4" />}
        title="Danger zone"
      >
        <Button
          onClick={onDeleteRepository}
          size="sm"
          type="button"
          variant="destructive"
        >
          <Trash2 className="size-3.5" />
          <span>Delete repository</span>
        </Button>
      </SettingsRow>
    </div>
  )
}

function SettingsRow({
  children,
  description,
  icon,
  title,
}: {
  children: ReactNode
  description: string
  icon: ReactNode
  title: string
}) {
  return (
    <section className="grid gap-4 py-5 md:grid-cols-[240px_minmax(0,1fr)]">
      <div className="min-w-0">
        <div className="flex items-center gap-2 text-sm font-semibold leading-5">
          {icon}
          <span>{title}</span>
        </div>
        <p className="mt-1 text-sm leading-5 text-muted-foreground">
          {description}
        </p>
      </div>
      <div className="min-w-0 md:pt-0.5">{children}</div>
    </section>
  )
}

function VisibilityChoice({
  current,
  disabled,
  onSelect,
}: {
  current: Visibility
  disabled: boolean
  onSelect: (visibility: Visibility) => void
}) {
  return (
    <div className="flex flex-wrap items-center gap-2">
      <Button
        aria-pressed={current === 'Private'}
        disabled={disabled}
        onClick={() => {
          if (current !== 'Private') {
            onSelect('Private')
          }
        }}
        size="sm"
        type="button"
        variant={current === 'Private' ? 'default' : 'secondary'}
      >
        <Lock className="size-3.5" />
        <span>Private</span>
      </Button>
      <Button
        aria-pressed={current === 'Public'}
        disabled={disabled}
        onClick={() => {
          if (current !== 'Public') {
            onSelect('Public')
          }
        }}
        size="sm"
        type="button"
        variant={current === 'Public' ? 'default' : 'secondary'}
      >
        <Globe2 className="size-3.5" />
        <span>Public</span>
      </Button>
      {disabled && (
        <LoaderCircle className="size-3.5 animate-spin text-muted-foreground" />
      )}
    </div>
  )
}

function repoSettingsPageReducer(
  state: RepoSettingsPageState,
  action: RepoSettingsPageAction,
): RepoSettingsPageState {
  switch (action.type) {
    case 'deleteFailed':
      return { ...state, deleteError: action.message }
    case 'deleteStarted':
      return { ...state, deleteError: null, deleteTarget: action.repo }
    case 'deleteTargetChanged':
      return { ...state, deleteTarget: action.repo }
    case 'gitCredentialStarted':
      return {
        ...state,
        gitCredentialError: null,
        gitCredentialPending: true,
      }
    case 'gitCredentialSucceeded':
      return {
        ...state,
        gitCredential: action.credential,
        gitCredentialPending: false,
      }
    case 'gitCredentialFailed':
      return {
        ...state,
        gitCredentialError: action.message,
        gitCredentialPending: false,
      }
    case 'settingsStarted':
      return {
        ...state,
        pendingSetting: action.key,
        settingsError: null,
      }
    case 'settingsSucceeded':
      return {
        ...state,
        pendingSetting: null,
        settingsOverride: {
          baseSettings: action.baseSettings,
          settings: action.settings,
        },
      }
    case 'settingsFailed':
      return {
        ...state,
        pendingSetting: null,
        settingsError: action.message,
      }
  }
}
