import type { RequestDiscussionReplyView } from './request-discussion-types'

export function replyFragment(discussionId: string, replyId: string) {
  const fragment = new URLSearchParams({
    discussion: discussionId,
    reply: replyId,
  })
  return `#${fragment.toString()}`
}

export function replyTargetFromFragment(hash: string) {
  if (!hash.startsWith('#')) return null
  const fragment = new URLSearchParams(hash.slice(1))
  const discussionId = fragment.get('discussion')
  const replyId = fragment.get('reply')
  if (!discussionId || !replyId) return null
  return { discussionId, replyId }
}

const GROUP_WINDOW_SECONDS = 5 * 60

export function shouldGroupReplies(
  previous: RequestDiscussionReplyView | null,
  current: RequestDiscussionReplyView,
  boundary: { date: boolean; unread: boolean },
) {
  return Boolean(
    previous &&
    !boundary.date &&
    !boundary.unread &&
    !previous.pending &&
    !current.pending &&
    previous.author.id === current.author.id &&
    current.created_at_unix >= previous.created_at_unix &&
    current.created_at_unix - previous.created_at_unix <=
      GROUP_WINDOW_SECONDS,
  )
}

export function sameUtcDate(leftUnix: number, rightUnix: number) {
  const left = new Date(leftUnix * 1_000)
  const right = new Date(rightUnix * 1_000)
  return (
    left.getUTCFullYear() === right.getUTCFullYear() &&
    left.getUTCMonth() === right.getUTCMonth() &&
    left.getUTCDate() === right.getUTCDate()
  )
}
