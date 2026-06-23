import type { HomeState, RepoSummary } from '@/api/types'

export type ThemeMode = 'dark' | 'light'

type HomeOverride = {
  account: HomeState['account']
  baseHome: HomeState
  repositories: RepoSummary[]
  signedIn: boolean
}

export type HomePageState = {
  createError: string | null
  homeOverride: HomeOverride | null
  sessionError: string | null
  theme: ThemeMode
}

export type HomePageAction =
  | { type: 'createFailed'; message: string }
  | { type: 'createStarted' }
  | {
      account: HomeState['account']
      baseHome: HomeState
      repositories: RepoSummary[]
      repo: RepoSummary
      signedIn: boolean
      type: 'repositoryCreated'
    }
  | { type: 'sessionFailed'; message: string }
  | { type: 'sessionStarted' }
  | { baseHome: HomeState; type: 'signedOut' }
  | { theme: ThemeMode; type: 'themeChanged' }

export const initialHomePageState: HomePageState = {
  createError: null,
  homeOverride: null,
  sessionError: null,
  theme: 'dark',
}

export function homePageReducer(
  state: HomePageState,
  action: HomePageAction,
): HomePageState {
  switch (action.type) {
    case 'createStarted':
      return { ...state, createError: null }
    case 'createFailed':
      return { ...state, createError: action.message }
    case 'repositoryCreated':
      return {
        ...state,
        homeOverride: {
          account: action.account,
          baseHome: action.baseHome,
          repositories: [action.repo, ...action.repositories],
          signedIn: action.signedIn,
        },
      }
    case 'sessionStarted':
      return { ...state, sessionError: null }
    case 'sessionFailed':
      return { ...state, sessionError: action.message }
    case 'signedOut':
      return {
        ...state,
        homeOverride: {
          account: null,
          baseHome: action.baseHome,
          repositories: [],
          signedIn: false,
        },
      }
    case 'themeChanged':
      return { ...state, theme: action.theme }
  }
}

export function activeHomeState(home: HomeState, state: HomePageState) {
  return state.homeOverride?.baseHome === home ? state.homeOverride : home
}

export function nextThemeMode(theme: ThemeMode): ThemeMode {
  return theme === 'dark' ? 'light' : 'dark'
}
