import type { RequestDiscussionReplyView } from './request-discussion-types'

export type ReplyPageState = {
  error: string | null
  loaded: boolean
  loading: boolean
  nextBeforePosition: number | null
  newestLoadedPosition: number | null
}

export type DiscussionRepliesState = {
  page: ReplyPageState
  replies: RequestDiscussionReplyView[]
}

export type ReplyPage = {
  next_before_position: number | null
  replies: RequestDiscussionReplyView[]
}

const unloadedPage: ReplyPageState = {
  error: null,
  loaded: false,
  loading: false,
  nextBeforePosition: null,
  newestLoadedPosition: null,
}

export function createDiscussionRepliesState(
  replies: RequestDiscussionReplyView[] = [],
): DiscussionRepliesState {
  return {
    page: unloadedPage,
    replies: mergeDiscussionReplies([], replies),
  }
}

export function mergeReplyPage(
  state: DiscussionRepliesState,
  page: ReplyPage,
  latest: RequestDiscussionReplyView[] = [],
  newestPage = false,
): DiscussionRepliesState {
  const replies = mergeDiscussionReplies(
    state.replies,
    [...latest, ...page.replies],
  )
  return {
    page: {
      error: null,
      loaded: true,
      loading: false,
      nextBeforePosition: page.next_before_position,
      newestLoadedPosition: newestPage
        ? page.replies.at(-1)?.position ?? state.page.newestLoadedPosition
        : state.page.newestLoadedPosition,
    },
    replies,
  }
}

export function beforePositionForNextReplyPage(
  state: DiscussionRepliesState,
  latest: RequestDiscussionReplyView[],
) {
  const newestPreviewPosition = latest.at(-1)?.position
  const newestPageIsStale =
    newestPreviewPosition !== undefined &&
    (state.page.newestLoadedPosition === null ||
      newestPreviewPosition > state.page.newestLoadedPosition)

  if (!state.page.loaded || newestPageIsStale) return undefined
  return state.page.nextBeforePosition ?? undefined
}

export function mergeReplyTarget(
  state: DiscussionRepliesState,
  page: ReplyPage,
  latest: RequestDiscussionReplyView[] = [],
): DiscussionRepliesState {
  return {
    page: { ...state.page, error: null, loading: false },
    replies: mergeDiscussionReplies(
      state.replies,
      [...latest, ...page.replies],
    ),
  }
}

export function insertOptimisticReply(
  state: DiscussionRepliesState,
  reply: RequestDiscussionReplyView,
  latest: RequestDiscussionReplyView[] = [],
): DiscussionRepliesState {
  const replies = mergeDiscussionReplies(state.replies, latest)
  return {
    ...state,
    page: { ...state.page, error: null },
    replies: mergeDiscussionReplies(replies, [reply]),
  }
}

export function markReplyFailed(
  state: DiscussionRepliesState,
  replyId: string,
): DiscussionRepliesState {
  if (!state.replies.some((reply) => reply.id === replyId)) return state
  return {
    ...state,
    replies: state.replies.map((reply) =>
      reply.id === replyId ? { ...reply, pending: 'failed' } : reply,
    ),
  }
}

export function acknowledgeReply(
  state: DiscussionRepliesState,
  optimisticReplyId: string,
  reply: RequestDiscussionReplyView,
): DiscussionRepliesState {
  const withoutOptimistic = state.replies.filter(
    (existing) => existing.id !== optimisticReplyId,
  )
  const acknowledged = { ...reply }
  delete acknowledged.pending
  return {
    ...state,
    replies: mergeDiscussionReplies(withoutOptimistic, [acknowledged]),
  }
}

export function mergeDiscussionReplies(
  current: RequestDiscussionReplyView[],
  latest: RequestDiscussionReplyView[],
) {
  const byId = new Map(current.map((reply) => [reply.id, reply]))
  for (const reply of latest) byId.set(reply.id, reply)
  return orderReplies(byId.values())
}

export function countVisibleUnreadReplies(
  replies: RequestDiscussionReplyView[],
  readThroughPosition: number,
) {
  return replies.filter(
    (reply) => !reply.pending && reply.position > readThroughPosition,
  ).length
}

export function hasLoadedAllUnreadContent(
  replies: RequestDiscussionReplyView[],
  readThroughPosition: number,
  unreadCount: number,
  rootUnread: boolean,
) {
  return (
    countVisibleUnreadReplies(replies, readThroughPosition) +
      Number(rootUnread) >=
    unreadCount
  )
}

export function updateReplyPage(
  state: DiscussionRepliesState,
  patch: Partial<ReplyPageState>,
): DiscussionRepliesState {
  return { ...state, page: { ...state.page, ...patch } }
}

function orderReplies(replies: Iterable<RequestDiscussionReplyView>) {
  return [...replies].sort((left, right) => left.position - right.position)
}
