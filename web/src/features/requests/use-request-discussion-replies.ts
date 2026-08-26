import { useRef, useState } from 'react'
import type {
  CreateReplyInput,
  LoadRepliesInput,
  RequestDiscussionRepliesPage,
} from './request-discussion-api'
import {
  acknowledgeReply,
  beforePositionForNextReplyPage,
  createDiscussionRepliesState,
  hasLoadedAllUnreadContent,
  insertOptimisticReply,
  loadReplyPagesThroughTarget,
  markReplyFailed,
  mergeDiscussionReplies,
  mergeReplyPage,
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
  const [quoteId, setQuoteId] = useState<string | null>(null)
  const targetLoadRef = useRef<{
    promise: Promise<boolean>
    replyId: string
  } | null>(null)

  const availableReplies = mergeDiscussionReplies(
    replyState.replies,
    discussion.latest_replies,
  )
  const loadingReplies = replyState.page.loading
  const replyError = replyState.page.error
  const loadedReplyCount = availableReplies.filter(
    (reply) => !reply.pending,
  ).length
  const olderReplyCount = Math.max(
    discussion.reply_count - loadedReplyCount,
    0,
  )
  const hasOlderReplies = olderReplyCount > 0

  async function loadReplyPage(before: number | undefined) {
    setReplyState((current) =>
      updateReplyPage(current, {
        error: null,
        loading: true,
      }),
    )
    try {
      const page = await actions.loadReplies({
        ...params,
        before,
        discussion_id: discussion.id,
      })
      setReplyState((current) =>
        mergeReplyPage(
          current,
          page,
          discussion.latest_replies,
          before === undefined,
        ),
      )
    } catch (error) {
      setReplyState((current) =>
        updateReplyPage(current, {
          error: messageFor(error, 'Earlier replies could not be loaded.'),
          loading: false,
        }),
      )
    }
  }

  function loadOlderReplies() {
    if (!hasOlderReplies || loadingReplies) return
    return loadReplyPage(
      beforePositionForNextReplyPage(
        replyState,
        discussion.latest_replies,
      ),
    )
  }

  function loadReplyTarget(replyId: string): Promise<boolean> {
    if (availableReplies.some((reply) => reply.id === replyId)) {
      return Promise.resolve(true)
    }
    if (targetLoadRef.current) {
      if (targetLoadRef.current.replyId === replyId) {
        return targetLoadRef.current.promise
      }
      return targetLoadRef.current.promise.then(() => loadReplyTarget(replyId))
    }

    setReplyState((current) =>
      updateReplyPage(current, { error: null, loading: true }),
    )
    const operation = (async () => {
      try {
        const pages = await loadReplyPagesThroughTarget(
          replyId,
          (before) =>
            actions.loadReplies({
              ...params,
              before,
              discussion_id: discussion.id,
            }),
        )
        setReplyState((current) =>
          pages.reduce(
            (state, page, index) =>
              mergeReplyPage(
                state,
                page,
                discussion.latest_replies,
                index === 0,
              ),
            current,
          ),
        )
        return pages.some((page) =>
          page.replies.some((reply) => reply.id === replyId),
        )
      } catch (error) {
        setReplyState((current) =>
          updateReplyPage(current, {
            error: messageFor(error, 'Linked reply could not be loaded.'),
            loading: false,
          }),
        )
        return false
      }
    })()
    targetLoadRef.current = { promise: operation, replyId }
    void operation.finally(() => {
      if (targetLoadRef.current?.promise === operation) {
        targetLoadRef.current = null
      }
    })
    return operation
  }

  async function postReply(
    body: string,
    clientReplyId: string = crypto.randomUUID(),
    replyToReplyId: string | null = quoteId,
    retryReference?: RequestDiscussionReplyView['reply_to'],
  ) {
    const replyTarget = retryReference ?? (
      replyToReplyId
        ? availableReplies.find((reply) => reply.id === replyToReplyId) ?? null
        : null
    )
    const optimistic = optimisticReply({
      actor,
      body,
      clientReplyId,
      discussion,
      replyTarget,
      replyToReplyId,
    })
    setReplyState((current) =>
      insertOptimisticReply(
        current,
        optimistic,
        discussion.latest_replies,
      ),
    )
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
        updateReplyPage(markReplyFailed(current, clientReplyId), {
          error: messageFor(error, 'Reply could not be posted.'),
        }),
      )
      return false
    }
  }

  const canPostReply =
    canReply && (discussion.status !== 'Resolved' || canResolve)
  const rootUnread =
    discussion.opened_position > discussion.read_through_position

  return {
    availableReplies,
    canPostReply,
    hasOlderReplies,
    loadOlderReplies,
    loadReplyTarget,
    loadingReplies,
    olderReplyCount,
    postReply,
    quotedReply: quoteId
      ? availableReplies.find((reply) => reply.id === quoteId) ?? null
      : null,
    replyError,
    setQuoteId,
    unreadContentFullyExposed:
      hasLoadedAllUnreadContent(
        availableReplies,
        discussion.read_through_position,
        discussion.unread_count,
        rootUnread,
      ),
  }
}

function optimisticReply({
  actor,
  body,
  clientReplyId,
  discussion,
  replyTarget,
  replyToReplyId,
}: {
  actor: { handle: string; id: string }
  body: string
  clientReplyId: string
  discussion: RequestDiscussion
  replyTarget: RequestDiscussionReplyView | RequestDiscussionReplyView['reply_to']
  replyToReplyId: string | null
}): RequestDiscussionReplyView {
  return {
    author: actor,
    body_markdown: body,
    created_at_unix: Math.floor(Date.now() / 1000),
    discussion_id: discussion.id,
    id: clientReplyId,
    optimistic_reply_to_reply_id: replyTarget ? undefined : replyToReplyId ?? undefined,
    pending: 'sending',
    position: Number.MAX_SAFE_INTEGER,
    reply_to: replyTarget
      ? {
          author: replyTarget.author,
          body_markdown: replyTarget.body_markdown,
          id: replyTarget.id,
          position: replyTarget.position,
        }
      : null,
  }
}

function messageFor(error: unknown, fallback: string) {
  return error instanceof Error && error.message.trim()
    ? error.message
    : fallback
}
