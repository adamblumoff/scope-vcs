import type { RequestDiscussionReplyView } from './request-discussion-types'

export type ReplyPageState = {
  error: string | null
  loaded: boolean
  loading: boolean
  nextBeforePosition: number | null
}

export type ReplyBranchState = ReplyPageState & {
  knownChildCount: number
}

export type DiscussionRepliesState = {
  branches: ReadonlyMap<string, ReplyBranchState>
  replies: RequestDiscussionReplyView[]
  root: ReplyPageState
}

type ReplyPage = {
  next_before_position: number | null
  replies: RequestDiscussionReplyView[]
}

const unloadedPage: ReplyPageState = {
  error: null,
  loaded: false,
  loading: false,
  nextBeforePosition: null,
}

export function createDiscussionRepliesState(
  replies: RequestDiscussionReplyView[] = [],
): DiscussionRepliesState {
  return {
    branches: new Map(),
    replies: mergeDiscussionReplies([], replies),
    root: unloadedPage,
  }
}

export function mergeReplyPage(
  state: DiscussionRepliesState,
  parentReplyId: string | null,
  page: ReplyPage,
  latest: RequestDiscussionReplyView[] = [],
): DiscussionRepliesState {
  const replies = mergeDiscussionReplies(
    state.replies,
    [...latest, ...page.replies],
  )
  const pageState: ReplyPageState = {
    error: null,
    loaded: true,
    loading: false,
    nextBeforePosition: page.next_before_position,
  }
  if (parentReplyId === null) {
    return { ...state, replies, root: pageState }
  }

  const parent = replies.find((reply) => reply.id === parentReplyId)
  const knownChildCount = Math.max(
    parent?.child_reply_count ?? 0,
    directDiscussionReplies(replies, parentReplyId).length,
  )
  const branches = new Map(state.branches)
  branches.set(parentReplyId, { ...pageState, knownChildCount })
  return { ...state, branches, replies }
}

export function insertOptimisticReply(
  state: DiscussionRepliesState,
  reply: RequestDiscussionReplyView,
  latest: RequestDiscussionReplyView[] = [],
): DiscussionRepliesState {
  const currentParent = reply.reply_to_reply_id === null
    ? undefined
    : state.replies.find((existing) => existing.id === reply.reply_to_reply_id)
  const replies = mergeDiscussionReplies(state.replies, latest)
  const alreadyPresent = replies.some((existing) => existing.id === reply.id)
  const withParentCount = replies.map((existing) => {
    if (existing.id !== reply.reply_to_reply_id) return existing
    const childReplyCount = alreadyPresent
      ? Math.max(existing.child_reply_count, currentParent?.child_reply_count ?? 0)
      : existing.child_reply_count + 1
    return { ...existing, child_reply_count: childReplyCount }
  })
  return {
    ...state,
    replies: mergeDiscussionReplies(withParentCount, [reply]),
    root: { ...state.root, error: null },
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

export function directDiscussionReplies(
  replies: RequestDiscussionReplyView[],
  parentReplyId: string | null,
) {
  return replies.filter(
    (reply) => reply.reply_to_reply_id === parentReplyId,
  )
}

export function replyTreeFullyExposed({
  expandedReplyIds,
  replyCount,
  rootRepliesLoaded,
  state,
}: {
  expandedReplyIds: ReadonlySet<string>
  replyCount: number
  rootRepliesLoaded: boolean
  state: DiscussionRepliesState
}) {
  return (
    rootRepliesLoaded &&
    state.root.nextBeforePosition === null &&
    state.replies.length >= replyCount &&
    state.replies.every((reply) =>
      reply.child_reply_count === 0 ||
      (
        expandedReplyIds.has(reply.id) &&
        state.branches.get(reply.id)?.loaded === true &&
        state.branches.get(reply.id)?.nextBeforePosition === null
      ),
    )
  )
}

export function updateReplyPage(
  state: DiscussionRepliesState,
  parentReplyId: string | null,
  patch: Partial<ReplyPageState>,
): DiscussionRepliesState {
  if (parentReplyId === null) {
    return { ...state, root: { ...state.root, ...patch } }
  }
  const branches = new Map(state.branches)
  branches.set(parentReplyId, {
    ...unloadedPage,
    knownChildCount: 0,
    ...branches.get(parentReplyId),
    ...patch,
  })
  return { ...state, branches }
}

function orderReplies(replies: Iterable<RequestDiscussionReplyView>) {
  return [...replies].sort((left, right) => left.position - right.position)
}
