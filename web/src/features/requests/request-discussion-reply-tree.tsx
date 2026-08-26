import { Badge } from '@/components/ui/badge'
import { cn } from '@/lib/utils'
import { CornerLeftUp, Reply, RotateCcw } from 'lucide-react'
import {
  RequestDiscussionActorAvatar,
  RequestDiscussionByline,
} from './request-discussion-byline'
import { RequestDiscussionMarkdown } from './request-discussion-markdown'
import { RequestDiscussionReplyToggle } from './request-discussion-reply-toggle'
import {
  directDiscussionReplies,
  type ReplyBranchState,
} from './request-discussion-replies-model'
import type { RequestDiscussionReplyView } from './request-discussion-types'

/**
 * Replies nest by sitting inside their parent's text column, so parentage is
 * structural. One nested level is all a 360px column affords; deeper replies
 * rejoin the parent column and name their target instead.
 */
const maxNestedDepth = 1

type ReplyTreeContext = {
  branchStates: ReadonlyMap<string, ReplyBranchState>
  canQuote: boolean
  expandedReplyIds: ReadonlySet<string>
  onLoadChildren: (replyId: string, before?: number) => void
  onQuote: (reply: RequestDiscussionReplyView) => void
  onRetry: (reply: RequestDiscussionReplyView) => void
  onToggleChildren: (reply: RequestDiscussionReplyView) => void
  replies: RequestDiscussionReplyView[]
}

export function RequestDiscussionReplyTree({
  visibleReplies,
  ...context
}: ReplyTreeContext & {
  visibleReplies: RequestDiscussionReplyView[]
}) {
  return visibleReplies.map((reply) => (
    <DiscussionReply
      context={context}
      depth={0}
      flattened={false}
      key={reply.id}
      reply={reply}
    />
  ))
}

function DiscussionReply({
  context,
  depth,
  flattened,
  reply,
}: {
  context: ReplyTreeContext
  depth: number
  flattened: boolean
  reply: RequestDiscussionReplyView
}) {
  const {
    branchStates,
    canQuote,
    expandedReplyIds,
    onLoadChildren,
    onQuote,
    onRetry,
    onToggleChildren,
    replies,
  } = context
  const children = directDiscussionReplies(replies, reply.id)
  const childCount = Math.max(reply.child_reply_count, children.length)
  const expanded = expandedReplyIds.has(reply.id)
  const branch = branchStates.get(reply.id)
  const branchCursor = branch?.nextBeforePosition ?? null
  const nested = depth < maxNestedDepth
  const parent = reply.reply_to_reply_id
    ? replies.find((candidate) => candidate.id === reply.reply_to_reply_id)
    : undefined

  return (
    <div
      className="grid scroll-mt-32 grid-cols-[1.25rem_minmax(0,1fr)] gap-x-2 py-3"
      id={`reply-${reply.id}`}
    >
      <RequestDiscussionActorAvatar handle={reply.author.handle} small />

      <div className="min-w-0">
        <RequestDiscussionByline
          author={reply.author}
          createdAtUnix={reply.created_at_unix}
          small
        >
          {parent && flattened ? (
            <span className="inline-flex items-center gap-1 text-xs text-muted-foreground">
              <CornerLeftUp className="size-3 shrink-0" />
              to {parent.author.handle}
            </span>
          ) : null}
          {reply.pending === 'sending' ? (
            <span className="text-xs text-muted-foreground">Posting…</span>
          ) : null}
          {reply.pending === 'failed' ? (
            <Badge variant="danger">Failed</Badge>
          ) : null}
        </RequestDiscussionByline>

        <RequestDiscussionMarkdown
          className="mt-1 max-w-[68ch]"
          source={reply.body_markdown}
        />

        <div className="mt-1.5 flex flex-wrap items-center gap-x-4 gap-y-1">
          {canQuote && reply.can_reply ? (
            <button
              className="inline-flex items-center gap-1 text-xs font-medium text-muted-foreground hover:text-foreground"
              onClick={() => onQuote(reply)}
              type="button"
            >
              <Reply className="size-3" />
              Reply
            </button>
          ) : null}
          {reply.pending === 'failed' ? (
            <button
              className="inline-flex items-center gap-1 text-xs font-medium text-destructive hover:text-foreground"
              onClick={() => onRetry(reply)}
              type="button"
            >
              <RotateCcw className="size-3" />
              Retry
            </button>
          ) : null}
          {childCount > 0 ? (
            <RequestDiscussionReplyToggle
              count={childCount}
              expanded={expanded}
              onToggle={() => onToggleChildren(reply)}
              subtle
            />
          ) : null}
        </div>

        {expanded ? (
          <div
            className={cn(
              nested
                ? 'border-l border-border/70 pl-3'
                // cancel the avatar gutter so a flattened reply costs no indent
                : '-ml-7',
            )}
          >
            {branch && branchCursor !== null ? (
              <button
                className="mt-3 text-xs font-medium text-muted-foreground hover:text-foreground"
                disabled={branch.loading}
                onClick={() => onLoadChildren(reply.id, branchCursor)}
                type="button"
              >
                {branch.loading ? 'Loading…' : 'Load older replies'}
              </button>
            ) : null}
            {children.map((child) => (
              <DiscussionReply
                context={context}
                depth={nested ? depth + 1 : depth}
                flattened={!nested}
                key={child.id}
                reply={child}
              />
            ))}
            {branch?.loading && children.length === 0 ? (
              <p className="py-2 text-xs text-muted-foreground">
                Loading replies…
              </p>
            ) : null}
            {branch?.error ? (
              <div className="flex items-center gap-2 py-2 text-xs" role="alert">
                <span className="text-destructive">{branch.error}</span>
                <button
                  className="font-medium text-foreground hover:underline"
                  onClick={() =>
                    onLoadChildren(reply.id, branchCursor ?? undefined)
                  }
                  type="button"
                >
                  Retry
                </button>
              </div>
            ) : null}
          </div>
        ) : null}
      </div>
    </div>
  )
}
