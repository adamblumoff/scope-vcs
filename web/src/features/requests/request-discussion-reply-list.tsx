import { Badge } from '@/components/ui/badge'
import { cn } from '@/lib/utils'
import { CornerLeftUp, Ellipsis, Reply, RotateCcw } from 'lucide-react'
import { compactDiscussionSummary } from './discussion-preview-text'
import {
  RequestDiscussionActorAvatar,
  RequestDiscussionByline,
} from './request-discussion-byline'
import { RequestDiscussionMarkdown } from './request-discussion-markdown'
import {
  replyFragment,
  sameUtcDate,
  shouldGroupReplies,
} from './request-discussion-reply-presentation'
import type { RequestDiscussionReplyView } from './request-discussion-types'

const DATE_FORMATTER = new Intl.DateTimeFormat('en-US', {
  day: 'numeric',
  month: 'long',
  timeZone: 'UTC',
  year: 'numeric',
})

export function RequestDiscussionReplyList({
  canReply,
  discussionCreatedAtUnix,
  onQuote,
  onRetry,
  readThroughPosition,
  replies,
  showUnreadBoundary,
}: {
  canReply: boolean
  discussionCreatedAtUnix: number
  onQuote: (reply: RequestDiscussionReplyView) => void
  onRetry: (reply: RequestDiscussionReplyView) => void
  readThroughPosition: number
  replies: RequestDiscussionReplyView[]
  showUnreadBoundary: boolean
}) {
  let previousCreatedAt = discussionCreatedAtUnix
  let previousReply: RequestDiscussionReplyView | null = null
  let unreadBoundaryRendered = false

  return replies.map((reply) => {
    const startsNewDate = !sameUtcDate(
      previousCreatedAt,
      reply.created_at_unix,
    )
    const startsUnread =
      showUnreadBoundary &&
      !unreadBoundaryRendered &&
      reply.position > readThroughPosition
    const grouped = shouldGroupReplies(previousReply, reply, {
      date: startsNewDate,
      unread: startsUnread,
    })

    if (startsUnread) unreadBoundaryRendered = true
    previousCreatedAt = reply.created_at_unix
    previousReply = reply

    return (
      <div key={reply.id}>
        {startsNewDate ? (
          <DiscussionBoundary label={DATE_FORMATTER.format(reply.created_at_unix * 1_000)} />
        ) : null}
        {startsUnread ? <DiscussionBoundary label="New" unread /> : null}
        <DiscussionReply
          canReply={canReply}
          grouped={grouped}
          onQuote={onQuote}
          onRetry={onRetry}
          reply={reply}
        />
      </div>
    )
  })
}

export function RequestDiscussionUnreadBoundary() {
  return <DiscussionBoundary label="New" unread />
}

function DiscussionReply({
  canReply,
  grouped,
  onQuote,
  onRetry,
  reply,
}: {
  canReply: boolean
  grouped: boolean
  onQuote: (reply: RequestDiscussionReplyView) => void
  onRetry: (reply: RequestDiscussionReplyView) => void
  reply: RequestDiscussionReplyView
}) {
  const canQuote = canReply && !reply.pending
  const hasActions = canQuote || reply.pending === 'failed'

  return (
    <div
      className={cn(
        'group/message relative grid scroll-mt-32 grid-cols-[2rem_minmax(0,1fr)] gap-x-3 rounded-md pr-10 hover:bg-muted/35 focus-within:bg-muted/35 lg:pr-16',
        grouped ? 'py-0.5' : 'pt-3 pb-1',
      )}
      id={`reply-${reply.id}`}
    >
      {grouped ? (
        <span aria-hidden="true" />
      ) : (
        <RequestDiscussionActorAvatar handle={reply.author.handle} />
      )}

      <div className="min-w-0">
        {grouped ? (
          <span className="sr-only">
            {reply.author.handle}, grouped message
          </span>
        ) : null}
        <ReplyReference reply={reply} />
        {!grouped ? (
          <RequestDiscussionByline
            author={reply.author}
            createdAtUnix={reply.created_at_unix}
          >
            {reply.pending === 'sending' ? (
              <span className="text-xs text-muted-foreground">Posting…</span>
            ) : null}
            {reply.pending === 'failed' ? (
              <Badge variant="danger">Failed</Badge>
            ) : null}
          </RequestDiscussionByline>
        ) : null}
        <RequestDiscussionMarkdown
          className={cn('max-w-[68ch]', grouped ? '' : 'mt-1')}
          source={reply.body_markdown}
        />
      </div>

      {hasActions ? (
        <div className="absolute top-1 right-1 hidden items-center rounded-md border border-border bg-background p-0.5 opacity-0 shadow-sm transition-opacity group-focus-within/message:opacity-100 group-hover/message:opacity-100 lg:flex">
          <ReplyActionItems
            canReply={canQuote}
            onQuote={onQuote}
            onRetry={onRetry}
            reply={reply}
          />
        </div>
      ) : null}

      {hasActions ? (
        <details className="group/actions absolute top-1 right-1 z-10 lg:hidden">
          <summary
            aria-label={`Actions for ${reply.author.handle}'s reply`}
            className="grid size-8 cursor-pointer list-none place-items-center rounded-md border border-border bg-background text-muted-foreground shadow-sm hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none [&::-webkit-details-marker]:hidden"
          >
            <Ellipsis className="size-4" />
          </summary>
          <div className="absolute top-9 right-0 flex min-w-32 flex-col rounded-md border border-border bg-background p-1 shadow-lg">
            <ReplyActionItems
              canReply={canQuote}
              labels
              onQuote={onQuote}
              onRetry={onRetry}
              reply={reply}
            />
          </div>
        </details>
      ) : null}
    </div>
  )
}

function ReplyActionItems({
  canReply,
  labels = false,
  onQuote,
  onRetry,
  reply,
}: {
  canReply: boolean
  labels?: boolean
  onQuote: (reply: RequestDiscussionReplyView) => void
  onRetry: (reply: RequestDiscussionReplyView) => void
  reply: RequestDiscussionReplyView
}) {
  const itemClass = cn(
    'rounded text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none',
    labels ? 'flex items-center gap-2 px-2 py-1.5 text-xs' : 'p-1.5',
  )
  return (
    <>
      {canReply ? (
        <button
          aria-label={`Reply to ${reply.author.handle}`}
          className={itemClass}
          onClick={(event) => {
            closeMobileActions(event.currentTarget)
            onQuote(reply)
          }}
          title={`Reply to ${reply.author.handle}`}
          type="button"
        >
          <Reply className="size-3.5" />
          {labels ? 'Reply' : null}
        </button>
      ) : null}
      {reply.pending === 'failed' ? (
        <button
          aria-label="Retry reply"
          className={cn(itemClass, 'text-destructive')}
          onClick={(event) => {
            closeMobileActions(event.currentTarget)
            onRetry(reply)
          }}
          title="Retry reply"
          type="button"
        >
          <RotateCcw className="size-3.5" />
          {labels ? 'Retry' : null}
        </button>
      ) : null}
    </>
  )
}

function closeMobileActions(element: HTMLElement) {
  element.closest('details')?.removeAttribute('open')
}

function ReplyReference({ reply }: { reply: RequestDiscussionReplyView }) {
  if (reply.reply_to) {
    return (
      <a
        className="mb-1 flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground"
        href={replyFragment(reply.discussion_id, reply.reply_to.id)}
      >
        <CornerLeftUp className="size-3 shrink-0" />
        <span className="shrink-0 font-medium text-foreground">
          {reply.reply_to.author.handle}
        </span>
        <span className="truncate">
          {compactDiscussionSummary(reply.reply_to.body_markdown)}
        </span>
      </a>
    )
  }
  if (!reply.optimistic_reply_to_reply_id) return null
  return (
    <span className="mb-1 flex items-center gap-1.5 text-xs text-muted-foreground">
      <CornerLeftUp className="size-3 shrink-0" />
      Replying to an earlier message
    </span>
  )
}

function DiscussionBoundary({
  label,
  unread = false,
}: {
  label: string
  unread?: boolean
}) {
  return (
    <div
      className={cn(
        'my-3 flex items-center gap-3 text-[11px] font-semibold',
        unread ? 'text-brand' : 'text-muted-foreground',
      )}
    >
      <hr
        aria-label={unread ? 'New replies' : `Messages from ${label}`}
        className={cn('h-px flex-1 border-0', unread ? 'bg-brand/60' : 'bg-border')}
      />
      <span>{label}</span>
      <span className={cn('h-px flex-1', unread ? 'bg-brand/60' : 'bg-border')} />
    </div>
  )
}
