import type { ReviewFile } from '@/api/types'

type FilesOverride = {
  baseFiles: ReviewFile[]
  files: ReviewFile[]
}

type PendingVisibility = {
  baseFiles: ReviewFile[]
  key: string
}

type VisibilityError = {
  baseFiles: ReviewFile[]
  message: string
}

export type RepoDetailPageState = {
  filesOverride: FilesOverride | null
  pendingVisibility: PendingVisibility | null
  visibilityError: VisibilityError | null
}

export type RepoDetailPageAction =
  | { baseFiles: ReviewFile[]; key: string; type: 'visibilityStarted' }
  | { baseFiles: ReviewFile[]; files: ReviewFile[]; type: 'visibilitySucceeded' }
  | { baseFiles: ReviewFile[]; message: string; type: 'visibilityFailed' }
  | { type: 'visibilityFinished' }

export const initialRepoDetailPageState: RepoDetailPageState = {
  filesOverride: null,
  pendingVisibility: null,
  visibilityError: null,
}

export function repoDetailPageReducer(
  state: RepoDetailPageState,
  action: RepoDetailPageAction,
): RepoDetailPageState {
  switch (action.type) {
    case 'visibilityStarted':
      return {
        ...state,
        pendingVisibility: {
          baseFiles: action.baseFiles,
          key: action.key,
        },
        visibilityError: null,
      }
    case 'visibilitySucceeded':
      return {
        ...state,
        filesOverride: {
          baseFiles: action.baseFiles,
          files: action.files,
        },
      }
    case 'visibilityFailed':
      return {
        ...state,
        visibilityError: {
          baseFiles: action.baseFiles,
          message: action.message,
        },
      }
    case 'visibilityFinished':
      return { ...state, pendingVisibility: null }
  }
}
