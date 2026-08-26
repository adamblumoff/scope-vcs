import { Badge } from '@/components/ui/badge'
import { CornerLeftUp, Reply, RotateCcw } from 'lucide-react'
import { RequestDiscussionByline } from './request-discussion-byline'
import { RequestDiscussionMarkdown } from './request-discussion-markdown'
import { RequestDiscussionReplyToggle } from './request-discussion-reply-toggle'
import {
  directDiscussionReplies,
  type ReplyBranchState,
} from './request-discussion-replies-model'
import type { RequestDiscussionReplyView } from './request-discussion-types'

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
  const parent = reply.reply_to_reply_id
    ? replies.find((candidate) => candidate.id === reply.reply_to_reply_id)
    : undefined

  return (
    <div className="scroll-mt-32 py-3" id={`reply-${reply.id}`}>
      <RequestDiscussionByline
        author={reply.author}
        createdAtUnix={reply.created_at_unix}
        small
      >
        {reply.pending === 'sending' ? (
          <span className="text-xs text-muted-foreground">Posting…</span>
        ) : null}
        {reply.pending === 'failed' ? (
          <Badge variant="danger">Failed</Badge>
        ) : null}
      </RequestDiscussionByline>

      {parent ? (
        <a
          className="mt-1.5 inline-flex max-w-full items-center gap-1.5 rounded-md bg-muted px-2 py-0.5 text-xs text-muted-foreground hover:text-foreground"
          href={`#reply-${parent.id}`}
        >
          <CornerLeftUp className="size-3 shrink-0" />
          <span className="truncate">Replying to {parent.author.handle}</span>
        </a>
      ) : null}

      <RequestDiscussionMarkdown
        className="mt-1.5"
        source={reply.body_markdown}
      />

      <div className="mt-2 flex flex-wrap items-center gap-2">
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
          <RequestDiscussionReplyToggle
            count={childCount}
            expanded={expanded}
            onToggle={() => onToggleChildren(reply)}
          />
        ) : null}
      </div>

      {expanded ? (
        <div>
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
