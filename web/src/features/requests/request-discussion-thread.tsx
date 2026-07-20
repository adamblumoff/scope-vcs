import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import {
  Check,
  ChevronDown,
  ChevronRight,
  CircleAlert,
  Link2,
  MessageSquare,
  Reply,
  RotateCcw,
} from 'lucide-react'
import { memo, type ReactNode, useEffect, useRef, useState } from 'react'
import type {
  CreateReplyInput,
  LoadRepliesInput,
  RequestDiscussionActionInput,
  RequestDiscussionRepliesPage,
} from './request-discussion-api'
import {
  RequestReplyComposer,
} from './request-discussion-composer'
import { RequestDiscussionMarkdown } from './request-discussion-markdown'
import type {
  RequestDiscussion,
  RequestDiscussionReplyMutation,
  RequestDiscussionReplyView,
  RequestDiscussionView,
} from './request-discussion-types'
import {
  compactDiscussionSummary,
  mergeDiscussionReplies,
  upsertDiscussionReply,
} from './request-discussion-model'
import { formatUnixDate } from './request-labels'

export type RequestDiscussionThreadActions = {
  createReply: (
    input: CreateReplyInput,
  ) => Promise<RequestDiscussionReplyMutation>
  loadReplies: (input: LoadRepliesInput) => Promise<RequestDiscussionRepliesPage>
  reopenAndReply: (
    input: CreateReplyInput,
  ) => Promise<RequestDiscussionReplyMutation>
}

export const RequestDiscussionThread = memo(function RequestDiscussionThread({
  actions,
  actor,
  canReply,
  canResolve,
  discussion,
  onExpandedChange,
  onMarkRead,
  onPatch,
  onRetryRoot,
  onSetResolved,
  params,
  rootContent,
}: {
  actions: RequestDiscussionThreadActions
  actor: { handle: string; id: string }
  canReply: boolean
  canResolve: boolean
  discussion: RequestDiscussionView
  onExpandedChange: (discussionId: string, expanded: boolean) => void
  onMarkRead: (discussion: RequestDiscussion) => Promise<void>
  onPatch: (discussion: RequestDiscussion) => void
  onRetryRoot: (discussion: RequestDiscussionView) => Promise<boolean>
  onSetResolved: (
    discussion: RequestDiscussion,
    resolved: boolean,
  ) => Promise<void>
  params: { owner: string; repo: string; request_id: string }
  rootContent?: ReactNode
}) {
  const [expandedReplies, setExpandedReplies] = useState(false)
  const [loadingReplies, setLoadingReplies] = useState(false)
  const [nextBeforePosition, setNextBeforePosition] = useState<number | null>(
    null,
  )
  const [replies, setReplies] = useState<RequestDiscussionReplyView[]>([])
  const [replyError, setReplyError] = useState<string | null>(null)
  const [quoteId, setQuoteId] = useState<string | null>(null)
  const articleRef = useRef<HTMLElement>(null)
  const collapsed =
    discussion.status === 'Resolved' &&
    Boolean(discussion.initiallyResolved) &&
    !discussion.expanded

  useEffect(() => {
    const article = articleRef.current
    if (
      !article ||
      collapsed ||
      discussion.unread_count === 0 ||
      discussion.reply_count > discussion.latest_replies.length ||
      discussion.pending
    ) return
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (!entry?.isIntersecting) return
        void onMarkRead(discussion)
        observer.disconnect()
      },
      { threshold: 0.6 },
    )
    observer.observe(article)
    return () => observer.disconnect()
  }, [collapsed, discussion, onMarkRead])

  async function expandReplies() {
    onExpandedChange(discussion.id, true)
    if (expandedReplies) {
      return
    }
    if (discussion.reply_count <= discussion.latest_replies.length) {
      setReplies(discussion.latest_replies)
      setExpandedReplies(true)
      void onMarkRead(discussion)
      return
    }
    setLoadingReplies(true)
    setReplyError(null)
    try {
      const page = await actions.loadReplies({
        ...params,
        discussion_id: discussion.id,
      })
      setReplies(page.replies)
      setNextBeforePosition(page.next_before_position)
      setExpandedReplies(true)
      void onMarkRead(discussion)
    } catch (error) {
      setReplyError(messageFor(error, 'Replies could not be loaded.'))
    } finally {
      setLoadingReplies(false)
    }
  }

  async function loadOlderReplies() {
    if (nextBeforePosition === null || loadingReplies) return
    setLoadingReplies(true)
    setReplyError(null)
    try {
      const page = await actions.loadReplies({
        ...params,
        before: nextBeforePosition,
        discussion_id: discussion.id,
      })
      setReplies((current) => [
        ...page.replies,
        ...current.filter(
          (reply) =>
            !page.replies.some((olderReply) => olderReply.id === reply.id),
        ),
      ])
      setNextBeforePosition(page.next_before_position)
    } catch (error) {
      setReplyError(messageFor(error, 'Older replies could not be loaded.'))
    } finally {
      setLoadingReplies(false)
    }
  }

  async function postReply(
    body: string,
    clientReplyId: string = crypto.randomUUID(),
    replyToReplyId: string | null = quoteId,
  ) {
    const optimistic = optimisticReply({
      body,
      clientReplyId,
      discussion,
      replyToReplyId,
      actor,
    })
    setReplies((current) =>
      upsertDiscussionReply(
        current,
        discussion.latest_replies,
        optimistic,
      ),
    )
    setExpandedReplies(true)
    setReplyError(null)
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
      setReplies((current) =>
        current.map((reply) =>
          reply.id === clientReplyId ? result.reply : reply,
        ),
      )
      onPatch(result.discussion)
      onExpandedChange(discussion.id, true)
      return true
    } catch (error) {
      setReplies((current) =>
        current.map((reply) =>
          reply.id === clientReplyId
            ? { ...reply, pending: 'failed' }
            : reply,
        ),
      )
      setReplyError(messageFor(error, 'Reply could not be posted.'))
      return false
    }
  }

  const availableReplies = mergeDiscussionReplies(
    replies,
    discussion.latest_replies,
  )
  const quotedReply = quoteId
    ? availableReplies.find((reply) => reply.id === quoteId) ?? null
    : null
  const replyLookup = new Map(
    availableReplies.map((reply) => [reply.id, reply]),
  )
  const visibleReplies: RequestDiscussionReplyView[] = expandedReplies
    ? availableReplies
    : discussion.latest_replies
  const canPostReply =
    canReply &&
    (
      discussion.status === 'Dormant' ||
      discussion.status === 'Open' ||
      canResolve
    )

  return (
    <article
      className="request-discussion-thread scroll-mt-32 border-t border-border px-5 py-5 first:border-t-0 lg:px-7"
      id={`discussion-${discussion.id}`}
      ref={articleRef}
    >
      <div className="flex min-w-0 items-start gap-3">
        <ActorAvatar handle={discussion.author.handle} />
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
            <span className="text-sm font-semibold">
              {discussion.author.handle}
            </span>
            {discussion.change_block ? (
              <span className="text-sm text-muted-foreground">pushed a code change</span>
            ) : null}
            <span className="font-mono text-xs tabular-nums text-muted-foreground">
              {formatUnixDate(discussion.created_at_unix)}
            </span>
            {discussion.unread_count > 0 ? (
              <Badge variant="info">
                {discussion.unread_count} new
              </Badge>
            ) : null}
            {discussion.pending === 'sending' ? (
              <span className="text-xs text-muted-foreground">Posting…</span>
            ) : null}
            {discussion.pending === 'failed' ? (
              <Badge variant="danger">Failed</Badge>
            ) : null}
            {!discussion.pending ? (
              <a
                aria-label="Link to discussion"
                className="text-muted-foreground hover:text-foreground"
                href={`#discussion-${discussion.id}`}
              >
                <Link2 className="size-3.5" />
              </a>
            ) : null}
          </div>

          {rootContent ?? (collapsed ? (
            <button
              className="mt-2 flex w-full min-w-0 items-start gap-2 text-left"
              onClick={() => {
                onExpandedChange(discussion.id, true)
                void onMarkRead(discussion)
              }}
              type="button"
            >
              <ChevronRight className="mt-1 size-3.5 shrink-0 text-muted-foreground" />
              <span className="line-clamp-2 text-sm leading-6">
                {compactDiscussionSummary(discussion.body_markdown)}
              </span>
            </button>
          ) : (
            discussion.body_markdown ? (
              <RequestDiscussionMarkdown
                className="mt-2"
                source={discussion.body_markdown}
              />
            ) : null
          ))}

          <div className="mt-3 flex flex-wrap items-center gap-2">
            {canPostReply || discussion.reply_count > 0 ? (
              <Button
                onClick={() => void expandReplies()}
                size="sm"
                type="button"
                variant="ghost"
              >
                <MessageSquare className="size-3.5" />
                {discussion.reply_count === 0
                  ? 'Reply'
                  : `${discussion.reply_count} ${discussion.reply_count === 1 ? 'reply' : 'replies'}`}
                {discussion.reply_count > 0 ? (
                  <ChevronDown className="size-3.5" />
                ) : null}
              </Button>
            ) : null}
            {canResolve && discussion.status !== 'Dormant' && !discussion.pending ? (
              <Button
                onClick={() =>
                  void onSetResolved(
                    discussion,
                    discussion.status === 'Open',
                  )
                }
                size="sm"
                type="button"
                variant="ghost"
              >
                {discussion.status === 'Open' ? (
                  <Check className="size-3.5" />
                ) : (
                  <RotateCcw className="size-3.5" />
                )}
                {discussion.status === 'Open' ? 'Resolve' : 'Reopen'}
              </Button>
            ) : null}
            {discussion.pending === 'failed' ? (
              <Button
                onClick={() => void onRetryRoot(discussion)}
                size="sm"
                type="button"
                variant="secondary"
              >
                <RotateCcw className="size-3.5" />
                Retry
              </Button>
            ) : null}
            {discussion.status === 'Resolved' ? (
              <span className="text-xs text-muted-foreground">
                Resolved
                {discussion.resolved_by
                  ? ` by ${discussion.resolved_by.handle}`
                  : ''}
              </span>
            ) : null}
          </div>

          {!collapsed && visibleReplies.length > 0 ? (
            <div className="mt-4 border-l border-border pl-4">
              {expandedReplies && nextBeforePosition !== null ? (
                <button
                  className="mb-3 text-xs font-medium text-muted-foreground hover:text-foreground"
                  disabled={loadingReplies}
                  onClick={() => void loadOlderReplies()}
                  type="button"
                >
                  {loadingReplies ? 'Loading…' : 'Load older replies'}
                </button>
              ) : null}
              {visibleReplies.map((reply) => (
                <DiscussionReply
                  canQuote={canPostReply}
                  key={reply.id}
                  onQuote={() => setQuoteId(reply.id)}
                  onRetry={
                    reply.pending === 'failed'
                      ? () =>
                          void postReply(
                            reply.body_markdown,
                            reply.id,
                            reply.reply_to_reply_id,
                          )
                      : undefined
                  }
                  quotedReply={
                    reply.reply_to_reply_id
                      ? replyLookup.get(reply.reply_to_reply_id) ?? null
                      : null
                  }
                  quotedReplyId={reply.reply_to_reply_id}
                  reply={reply}
                />
              ))}
            </div>
          ) : null}

          {loadingReplies ? (
            <p className="mt-3 text-xs text-muted-foreground">
              Loading replies…
            </p>
          ) : null}
          {replyError ? (
            <p
              className="mt-3 flex items-center gap-2 text-sm text-destructive"
              role="alert"
            >
              <CircleAlert className="size-4" />
              {replyError}
            </p>
          ) : null}

          {!collapsed && canPostReply ? (
            <div className="mt-4">
              <RequestReplyComposer
                onCancelQuote={() => setQuoteId(null)}
                onSubmit={postReply}
                quote={
                  quotedReply
                    ? {
                        author: quotedReply.author.handle,
                        body: compactDiscussionSummary(
                          quotedReply.body_markdown,
                        ),
                      }
                    : null
                }
                reopen={discussion.status === 'Resolved'}
              />
            </div>
          ) : null}
        </div>
      </div>
    </article>
  )
})

function DiscussionReply({
  canQuote,
  onQuote,
  onRetry,
  quotedReply,
  quotedReplyId,
  reply,
}: {
  canQuote: boolean
  onQuote: () => void
  onRetry?: () => void
  quotedReply: RequestDiscussionReplyView | null
  quotedReplyId: string | null
  reply: RequestDiscussionReplyView
}) {
  return (
    <div
      className="scroll-mt-32 border-t border-border/70 py-4 first:border-t-0 first:pt-0"
      id={`reply-${reply.id}`}
    >
      <div className="flex items-center gap-2">
        <ActorAvatar handle={reply.author.handle} small />
        <span className="text-sm font-semibold">{reply.author.handle}</span>
        <span className="font-mono text-xs tabular-nums text-muted-foreground">
          {formatUnixDate(reply.created_at_unix)}
        </span>
        {reply.pending === 'sending' ? (
          <span className="text-xs text-muted-foreground">Posting…</span>
        ) : null}
        {reply.pending === 'failed' ? (
          <Badge variant="danger">Failed</Badge>
        ) : null}
      </div>
      {quotedReply || quotedReplyId ? (
        <div className="mt-2 border-l-2 border-border-strong pl-3 text-xs leading-5 text-muted-foreground">
          {quotedReply ? (
            <>
              <span className="font-medium text-foreground">
                {quotedReply.author.handle}
              </span>
              <span className="ml-1">
                {compactDiscussionSummary(quotedReply.body_markdown)}
              </span>
            </>
          ) : (
            <span>Replying to an earlier message</span>
          )}
        </div>
      ) : null}
      <RequestDiscussionMarkdown
        className="mt-1.5"
        source={reply.body_markdown}
      />
      <div className="mt-2 flex items-center gap-2">
        {canQuote ? (
          <button
            className="inline-flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
            onClick={onQuote}
            type="button"
          >
            <Reply className="size-3" />
            Reply
          </button>
        ) : null}
        {onRetry ? (
          <button
            className="inline-flex items-center gap-1 text-xs text-destructive hover:text-foreground"
            onClick={onRetry}
            type="button"
          >
            <RotateCcw className="size-3" />
            Retry
          </button>
        ) : null}
      </div>
    </div>
  )
}

function ActorAvatar({
  handle,
  small = false,
}: {
  handle: string
  small?: boolean
}) {
  return (
    <div
      aria-hidden="true"
      className={cn(
        'grid shrink-0 place-items-center rounded-full border border-border bg-muted font-mono font-semibold uppercase text-muted-foreground',
        small ? 'size-6 text-[9px]' : 'size-8 text-[10px]',
      )}
    >
      {handle.slice(0, 2)}
    </div>
  )
}

function optimisticReply({
  body,
  clientReplyId,
  discussion,
  replyToReplyId,
  actor,
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
