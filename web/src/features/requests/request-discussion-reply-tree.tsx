import { Badge } from '@/components/ui/badge'
import { cn } from '@/lib/utils'
import { Reply, RotateCcw } from 'lucide-react'
import { RequestDiscussionMarkdown } from './request-discussion-markdown'
import {
  directDiscussionReplies,
  type ReplyBranchState,
} from './request-discussion-replies-model'
import type { RequestDiscussionReplyView } from './request-discussion-types'
import { formatUnixDate } from './request-labels'

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
    <DiscussionReply context={context} key={reply.id} reply={reply} />
  ))
}

function DiscussionReply({
  context,
  reply,
}: {
  context: ReplyTreeContext
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

  return (
    <div
      className="scroll-mt-32 border-t border-border/70 py-4 first:border-t-0 first:pt-0"
      id={`reply-${reply.id}`}
    >
      <div className="flex items-center gap-2">
        <RequestDiscussionActorAvatar handle={reply.author.handle} small />
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
      <RequestDiscussionMarkdown
        className="mt-1.5"
        source={reply.body_markdown}
      />
      <div className="mt-2 flex items-center gap-2">
        {canQuote && reply.can_reply ? (
          <button
            className="inline-flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
            onClick={() => onQuote(reply)}
            type="button"
          >
            <Reply className="size-3" />
            Reply
          </button>
        ) : null}
        {reply.pending === 'failed' ? (
          <button
            className="inline-flex items-center gap-1 text-xs text-destructive hover:text-foreground"
            onClick={() => onRetry(reply)}
            type="button"
          >
            <RotateCcw className="size-3" />
            Retry
          </button>
        ) : null}
        {childCount > 0 ? (
          <button
            aria-label={expanded
              ? 'Hide replies'
              : `Show ${childCount} ${childCount === 1 ? 'reply' : 'replies'}`}
            aria-expanded={expanded}
            className="text-xs font-medium text-brand hover:text-foreground"
            onClick={() => onToggleChildren(reply)}
            type="button"
          >
            {expanded ? '-' : `+${childCount}`}
          </button>
        ) : null}
      </div>

      {expanded ? (
        <div className="mt-3 border-l border-border pl-4">
          {branch && branchCursor !== null ? (
            <button
              className="mb-3 text-xs font-medium text-muted-foreground hover:text-foreground"
              disabled={branch.loading}
              onClick={() => onLoadChildren(reply.id, branchCursor)}
              type="button"
            >
              {branch.loading ? 'Loading…' : 'Load older replies'}
            </button>
          ) : null}
          {children.map((child) => (
            <DiscussionReply context={context} key={child.id} reply={child} />
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
  )
}

export function RequestDiscussionActorAvatar({
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
