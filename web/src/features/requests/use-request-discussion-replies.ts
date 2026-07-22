import { useState } from 'react'
import type {
  CreateReplyInput,
  LoadRepliesInput,
  RequestDiscussionRepliesPage,
} from './request-discussion-api'
import {
  acknowledgeReply,
  createDiscussionRepliesState,
  directDiscussionReplies,
  insertOptimisticReply,
  markReplyFailed,
  mergeDiscussionReplies,
  mergeReplyPage,
  replyTreeFullyExposed,
  updateReplyPage,
} from './request-discussion-replies-model'
import type {
  RequestDiscussion,
  RequestDiscussionReplyMutation,
  RequestDiscussionReplyView,
  RequestDiscussionView,
} from './request-discussion-types'

export type RequestDiscussionThreadActions = {
  createReply: (
    input: CreateReplyInput,
  ) => Promise<RequestDiscussionReplyMutation>
  loadReplies: (input: LoadRepliesInput) => Promise<RequestDiscussionRepliesPage>
  reopenAndReply: (
    input: CreateReplyInput,
  ) => Promise<RequestDiscussionReplyMutation>
}

export function useRequestDiscussionReplies({
  actions,
  actor,
  canReply,
  canResolve,
  discussion,
  onExpandedChange,
  onPatch,
  params,
}: {
  actions: RequestDiscussionThreadActions
  actor: { handle: string; id: string }
  canReply: boolean
  canResolve: boolean
  discussion: RequestDiscussionView
  onExpandedChange: (discussionId: string, expanded: boolean) => void
  onPatch: (discussion: RequestDiscussion) => void
  params: { owner: string; repo: string; request_id: string }
}) {
  const [replyState, setReplyState] = useState(() =>
    createDiscussionRepliesState(),
  )
  const [expandedReplies, setExpandedReplies] = useState(false)
  const [expandedReplyIds, setExpandedReplyIds] = useState<Set<string>>(
    new Set(),
  )
  const [quoteId, setQuoteId] = useState<string | null>(null)

  const availableReplies = mergeDiscussionReplies(
    replyState.replies,
    discussion.latest_replies,
  )
  const loadingReplies = replyState.root.loading
  const nextBeforePosition = replyState.root.nextBeforePosition
  const replyBranches = replyState.branches
  const replyError = replyState.root.error

  async function loadReplyPage(
    parentReplyId: string | null,
    before: number | undefined,
    failureMessage: string,
  ) {
    setReplyState((current) =>
      updateReplyPage(current, parentReplyId, {
        error: null,
        loading: true,
      }),
    )
    try {
      const page = await actions.loadReplies({
        ...params,
        before,
        discussion_id: discussion.id,
        parent_reply_id: parentReplyId ?? undefined,
      })
      setReplyState((current) =>
        mergeReplyPage(
          current,
          parentReplyId,
          page,
          discussion.latest_replies,
        ),
      )
    } catch (error) {
      setReplyState((current) =>
        updateReplyPage(current, parentReplyId, {
          error: messageFor(error, failureMessage),
          loading: false,
        }),
      )
    }
  }

  function expandReplies() {
    onExpandedChange(discussion.id, true)
    setExpandedReplies(true)
    if (
      replyState.root.loaded ||
      replyState.root.loading ||
      discussion.reply_count === 0
    ) return
    return loadReplyPage(null, undefined, 'Replies could not be loaded.')
  }

  function loadOlderReplies() {
    if (nextBeforePosition === null || loadingReplies) return
    return loadReplyPage(
      null,
      nextBeforePosition,
      'Older replies could not be loaded.',
    )
  }

  function loadReplyChildren(parentReplyId: string, before?: number) {
    return loadReplyPage(parentReplyId, before, 'Replies could not be loaded.')
  }

  async function toggleReplyChildren(reply: RequestDiscussionReplyView) {
    if (expandedReplyIds.has(reply.id)) {
      setExpandedReplyIds((current) => {
        const next = new Set(current)
        next.delete(reply.id)
        return next
      })
      return
    }
    setExpandedReplyIds((current) => new Set(current).add(reply.id))
    const branch = replyBranches.get(reply.id)
    if (
      branch?.loaded &&
      branch.knownChildCount >= reply.child_reply_count
    ) return
    await loadReplyChildren(reply.id)
  }

  async function postReply(
    body: string,
    clientReplyId: string = crypto.randomUUID(),
    replyToReplyId: string | null = quoteId,
  ) {
    const optimistic = optimisticReply({
      actor,
      body,
      clientReplyId,
      discussion,
      replyToReplyId,
    })
    setReplyState((current) =>
      insertOptimisticReply(
        current,
        optimistic,
        discussion.latest_replies,
      ),
    )
    if (replyToReplyId) {
      setExpandedReplyIds((current) => new Set(current).add(replyToReplyId))
    }
    setExpandedReplies(true)
    const input = {
      ...params,
      body_markdown: body,
      client_reply_id: clientReplyId,
      discussion_id: discussion.id,
      reply_to_reply_id: replyToReplyId,
    }
    try {
      const result = await (
        discussion.status === 'Resolved'
          ? actions.reopenAndReply(input)
          : actions.createReply(input)
      )
      setReplyState((current) =>
        acknowledgeReply(current, clientReplyId, result.reply),
      )
      onPatch(result.discussion)
      onExpandedChange(discussion.id, true)
      setQuoteId(null)
      return true
    } catch (error) {
      setReplyState((current) =>
        updateReplyPage(
          markReplyFailed(current, clientReplyId),
          null,
          { error: messageFor(error, 'Reply could not be posted.') },
        ),
      )
      return false
    }
  }

  const visibleReplies = expandedReplies
    ? directDiscussionReplies(availableReplies, null)
    : directDiscussionReplies(discussion.latest_replies, null)
  const canPostReply =
    canReply && (discussion.status !== 'Resolved' || canResolve)
  const entireReplyTreeExposed = replyTreeFullyExposed({
    expandedReplyIds,
    replyCount: discussion.reply_count,
    rootRepliesLoaded:
      replyState.root.loaded ||
      discussion.latest_replies.length >= discussion.reply_count,
    state: { ...replyState, replies: availableReplies },
  })
  const previewContentExposed =
    discussion.reply_count <= discussion.latest_replies.length &&
    discussion.latest_replies.every(
      (reply) => reply.reply_to_reply_id === null,
    )

  return {
    availableReplies,
    canPostReply,
    entireReplyTreeExposed,
    expandedReplies,
    expandedReplyIds,
    expandReplies,
    loadOlderReplies,
    loadReplyChildren,
    loadingReplies,
    nextBeforePosition,
    postReply,
    previewContentExposed,
    quotedReply: quoteId
      ? availableReplies.find((reply) => reply.id === quoteId) ?? null
      : null,
    replyBranches,
    replyError,
    setQuoteId,
    toggleReplyChildren,
    visibleReplies,
  }
}

function optimisticReply({
  actor,
  body,
  clientReplyId,
  discussion,
  replyToReplyId,
}: {
  actor: { handle: string; id: string }
  body: string
  clientReplyId: string
  discussion: RequestDiscussion
  replyToReplyId: string | null
}): RequestDiscussionReplyView {
  return {
    author: actor,
    body_markdown: body,
    child_reply_count: 0,
    can_reply: false,
    created_at_unix: Math.floor(Date.now() / 1000),
    discussion_id: discussion.id,
    id: clientReplyId,
    pending: 'sending',
    position: Number.MAX_SAFE_INTEGER,
    reply_to_reply_id: replyToReplyId,
  }
}

function messageFor(error: unknown, fallback: string) {
  return error instanceof Error && error.message.trim()
    ? error.message
    : fallback
}
