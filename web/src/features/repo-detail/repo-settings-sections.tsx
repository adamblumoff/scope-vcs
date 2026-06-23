import type {
  RepoGitCredentialView,
  RepoSettings,
  Visibility,
} from '@/api/types'
import { CopyableCodeBlock } from '@/components/copyable-code-block'
import { Button } from '@/components/ui/button'
import { Separator } from '@/components/ui/separator'
import { Switch } from '@/components/ui/switch'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import {
  FilePlus2,
  GitBranch,
  Globe2,
  KeyRound,
  LoaderCircle,
  Lock,
  RefreshCw,
  Trash2,
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
    <div className="mt-8 border-y border-border">
      <SettingsRow
        description="Future Git pushes either stop in review or apply directly to the live repo."
        icon={<GitBranch className="size-4" />}
        title="Push workflow"
      >
        <div className="flex items-center gap-3 text-sm leading-5">
          <Switch
            aria-label="Review pushes before applying"
            checked={settings.review_pushes_before_applying}
            disabled={settingsSaving}
            onCheckedChange={() =>
              onSaveSettings(
                {
                  ...settings,
                  review_pushes_before_applying:
                    !settings.review_pushes_before_applying,
                },
                'push-review',
              )
            }
            type="button"
          />
          <span>Review pushes before applying</span>
          {pendingSetting === 'push-review' && (
            <LoaderCircle className="size-3.5 animate-spin text-muted-foreground" />
          )}
        </div>
      </SettingsRow>
      <Separator />

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
      <Separator />

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
      <Separator />

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
      <ToggleGroup
        disabled={disabled}
        onValueChange={(visibility) => {
          if (visibility && visibility !== current) {
            onSelect(visibility as Visibility)
          }
        }}
        type="single"
        value={current}
      >
        <ToggleGroupItem
          aria-label="Set default new files private"
          value="Private"
        >
          <Lock className="size-3.5" />
          <span>Private</span>
        </ToggleGroupItem>
        <ToggleGroupItem
          aria-label="Set default new files public"
          value="Public"
        >
          <Globe2 className="size-3.5" />
          <span>Public</span>
        </ToggleGroupItem>
      </ToggleGroup>
      {disabled && (
        <LoaderCircle className="size-3.5 animate-spin text-muted-foreground" />
      )}
    </div>
  )
}
