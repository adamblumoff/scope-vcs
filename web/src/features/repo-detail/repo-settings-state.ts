import type { RepoSettings, RepoSummary } from '@/api/types'

export type SettingKey = 'default-new-file-visibility' | 'push-review'

type SettingsOverride = {
  baseSettings: RepoSettings
  settings: RepoSettings
}

export type RepoSettingsPageState = {
  deleteError: string | null
  deleteTarget: RepoSummary | null
  pendingSetting: SettingKey | null
  settingsError: string | null
  settingsOverride: SettingsOverride | null
}

export type RepoSettingsPageAction =
  | { type: 'deleteFailed'; message: string }
  | { type: 'deleteStarted'; repo: RepoSummary }
  | { type: 'deleteTargetChanged'; repo: RepoSummary | null }
  | { key: SettingKey; type: 'settingsStarted' }
  | {
      baseSettings: RepoSettings
      settings: RepoSettings
      type: 'settingsSucceeded'
    }
  | { message: string; type: 'settingsFailed' }

export const initialRepoSettingsPageState: RepoSettingsPageState = {
  deleteError: null,
  deleteTarget: null,
  pendingSetting: null,
  settingsError: null,
  settingsOverride: null,
}

export function repoSettingsPageReducer(
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
