import type { RepoReview } from '@/api/types'

type ReviewOverride = {
  baseReview: RepoReview
  review: RepoReview
}

export type ReviewPageState = {
  error: string | null
  pendingKey: string | null
  reviewOverride: ReviewOverride | null
  runningAction: 'publish' | 'reject' | null
}

export type ReviewPageAction =
  | { type: 'actionFailed'; message: string }
  | { type: 'publishStarted' }
  | { type: 'rejectStarted' }
  | { type: 'visibilityFailed'; message: string }
  | { type: 'visibilityFinished' }
  | { type: 'visibilityStarted'; pendingKey: string }
  | { baseReview: RepoReview; review: RepoReview; type: 'visibilitySucceeded' }

export const initialReviewPageState: ReviewPageState = {
  error: null,
  pendingKey: null,
  reviewOverride: null,
  runningAction: null,
}

export function reviewPageReducer(
  state: ReviewPageState,
  action: ReviewPageAction,
): ReviewPageState {
  switch (action.type) {
    case 'actionFailed':
      return { ...state, error: action.message, runningAction: null }
    case 'publishStarted':
      return { ...state, error: null, runningAction: 'publish' }
    case 'rejectStarted':
      return { ...state, error: null, runningAction: 'reject' }
    case 'visibilityFailed':
      return { ...state, error: action.message }
    case 'visibilityFinished':
      return { ...state, pendingKey: null }
    case 'visibilityStarted':
      return { ...state, error: null, pendingKey: action.pendingKey }
    case 'visibilitySucceeded':
      return {
        ...state,
        reviewOverride: {
          baseReview: action.baseReview,
          review: action.review,
        },
      }
  }
}
