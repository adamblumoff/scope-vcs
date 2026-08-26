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

/**
 * The thread's reply control reveals root-level replies only, so it compares
 * against the root count. Comparing against the whole tree renders a control
 * that reveals nothing when every remaining reply is nested behind its own.
 */
export function showsThreadReplyToggle({
  expanded,
  rootReplyCount,
  visibleCount,
}: {
  expanded: boolean
  rootReplyCount: number
  visibleCount: number
}) {
  return expanded || rootReplyCount > visibleCount
}
