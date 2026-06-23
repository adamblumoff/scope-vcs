import type {
  RepoGitCredentialView,
  RepoSettings,
  RepoSummary,
} from '@/api/types'

export type SettingKey = 'default-new-file-visibility' | 'push-review'

type SettingsOverride = {
  baseSettings: RepoSettings
  settings: RepoSettings
}

export type RepoSettingsPageState = {
  deleteError: string | null
  deleteTarget: RepoSummary | null
  gitCredential: RepoGitCredentialView | null
  gitCredentialError: string | null
  gitCredentialPending: boolean
  pendingSetting: SettingKey | null
  settingsError: string | null
  settingsOverride: SettingsOverride | null
}

export type RepoSettingsPageAction =
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

export const initialRepoSettingsPageState: RepoSettingsPageState = {
  deleteError: null,
  deleteTarget: null,
  gitCredential: null,
  gitCredentialError: null,
  gitCredentialPending: false,
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
