import type {
  RepoGitCredentialView,
  RepoSettings,
  Visibility,
} from '@/api/types'
import { CopyableCodeBlock } from '@/components/copyable-code-block'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
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
import { gitCredentialApproveCommand } from '../setup/commands'
import type { SettingKey } from './repo-settings-state'

export function SettingsSections({
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
