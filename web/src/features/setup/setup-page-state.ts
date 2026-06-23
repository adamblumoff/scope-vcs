import type { RepoSetupView, SetupProgressState } from '@/api/types'

type SetupOverride = {
  baseSetup: RepoSetupView
  pushTokenSecret: string | null
  setup: RepoSetupView
}

export type SetupPageState = {
  busy: boolean
  error: string | null
  progressError: string | null
  progressState: SetupProgressState
  setupOverride: SetupOverride | null
}

export type SetupPageAction =
  | { message: string; type: 'progressFailed' }
  | { progressState: SetupProgressState; type: 'progressStateChanged' }
  | { type: 'tokenFailed'; message: string }
  | {
      baseSetup: RepoSetupView
      pushTokenSecret: string | null
      setup: RepoSetupView
      type: 'tokenSucceeded'
    }
  | { type: 'tokenStarted' }

export const initialSetupPageState: SetupPageState = {
  busy: false,
  error: null,
  progressError: null,
  progressState: 'waiting',
  setupOverride: null,
}

export function setupPageReducer(
  state: SetupPageState,
  action: SetupPageAction,
): SetupPageState {
  switch (action.type) {
    case 'progressFailed':
      return { ...state, progressError: action.message }
    case 'progressStateChanged':
      return {
        ...state,
        progressError: null,
        progressState: action.progressState,
      }
    case 'tokenFailed':
      return { ...state, busy: false, error: action.message }
    case 'tokenStarted':
      return { ...state, busy: true, error: null }
    case 'tokenSucceeded':
      return {
        ...state,
        busy: false,
        setupOverride: {
          baseSetup: action.baseSetup,
          pushTokenSecret: action.pushTokenSecret,
          setup: action.setup,
        },
      }
  }
}
