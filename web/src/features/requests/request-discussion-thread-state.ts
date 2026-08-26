import type { RequestDiscussionView } from './request-discussion-types'

/**
 * The single visual state a thread is in. The thread rail and the header chip
 * both read from this so they can never disagree.
 */
export type RequestDiscussionThreadState =
  | 'failed'
  | 'read'
  | 'resolved'
  | 'unread'

export function requestDiscussionThreadState(
  discussion: RequestDiscussionView,
): RequestDiscussionThreadState {
  if (discussion.pending === 'failed') return 'failed'
  if (discussion.status === 'Resolved') return 'resolved'
  if (discussion.unread_count > 0) return 'unread'
  return 'read'
}

const railColors: Record<RequestDiscussionThreadState, string> = {
  failed: 'bg-danger-border',
  read: 'bg-border',
  resolved: 'bg-success-border',
  unread: 'bg-brand',
}

export function requestDiscussionRailColor(
  state: RequestDiscussionThreadState,
) {
  return railColors[state]
}
