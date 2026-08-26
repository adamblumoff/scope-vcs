import type { RequestDiscussionView } from './request-discussion-types'

export function requestDiscussionRailColor(
  discussion: Pick<
    RequestDiscussionView,
    'pending' | 'status' | 'unread_count'
  >,
) {
  if (discussion.pending === 'failed') return 'bg-danger-border'
  if (discussion.status === 'Resolved') return 'bg-success-border'
  if (discussion.unread_count > 0) return 'bg-brand'
  return 'bg-border'
}
