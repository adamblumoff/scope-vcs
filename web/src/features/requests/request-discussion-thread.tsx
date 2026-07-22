import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import {
  Check,
  ChevronDown,
  ChevronRight,
  CircleAlert,
  GitCommitHorizontal,
  Link2,
  MessageSquare,
  RotateCcw,
} from 'lucide-react'
import {
  memo,
  type ReactNode,
  useEffect,
  useEffectEvent,
  useRef,
  useState,
} from 'react'
import {
  RequestReplyComposer,
} from './request-discussion-composer'
import { RequestDiscussionMarkdown } from './request-discussion-markdown'
import {
  RequestDiscussionActorAvatar,
  RequestDiscussionReplyTree,
} from './request-discussion-reply-tree'
import {
  useRequestDiscussionReplies,
  type RequestDiscussionThreadActions,
} from './use-request-discussion-replies'
import type {
  RequestDiscussion,
  RequestDiscussionView,
} from './request-discussion-types'
import { compactDiscussionSummary } from './request-discussion-model'
import { formatUnixDate } from './request-labels'

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
  const [readMarkerVisible, setReadMarkerVisible] = useState(false)
  const readMarkerRef = useRef<HTMLSpanElement>(null)
  const markReadAttemptRef = useRef<number | null>(null)
  const collapsed =
    discussion.status === 'Resolved' &&
    Boolean(discussion.initiallyResolved) &&
    !discussion.expanded
  const {
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
    quotedReply,
    replyBranches,
    replyError,
    setQuoteId,
    toggleReplyChildren,
    visibleReplies,
  } = useRequestDiscussionReplies({
    actions,
    actor,
    canReply,
    canResolve,
    discussion,
    onExpandedChange,
    onPatch,
    params,
  })

  useEffect(() => {
    const marker = readMarkerRef.current
    if (!marker || collapsed) return
    const observer = new IntersectionObserver(
      ([entry]) => {
        const visible = Boolean(entry?.isIntersecting)
        if (!visible) markReadAttemptRef.current = null
        setReadMarkerVisible(visible)
      },
      { threshold: 1 },
    )
    observer.observe(marker)
    return () => observer.disconnect()
  }, [collapsed])

  const markRead = useEffectEvent(onMarkRead)

  useEffect(() => {
    if (
      !readMarkerVisible ||
      collapsed ||
      discussion.unread_count === 0 ||
      discussion.pending ||
      (!previewContentExposed && !entireReplyTreeExposed)
    ) return
    if (
      markReadAttemptRef.current === discussion.last_activity_position
    ) return
    markReadAttemptRef.current = discussion.last_activity_position
    void markRead(discussion)
  }, [
    readMarkerVisible,
    collapsed,
    discussion,
    entireReplyTreeExposed,
    previewContentExposed,
  ])

  return (
    <article
      className={cn(
        'request-discussion-thread scroll-mt-32 border-t px-5 first:border-t-0 lg:px-7',
        discussion.change_block
          ? 'border-brand/25 bg-brand-muted/45 py-4 shadow-[inset_3px_0_0_0_var(--brand)]'
          : 'border-border py-5',
      )}
      id={`discussion-${discussion.id}`}
    >
      <div className="flex min-w-0 items-start gap-3">
        {discussion.change_block ? (
          <ChangeEventMarker />
        ) : (
          <RequestDiscussionActorAvatar handle={discussion.author.handle} />
        )}
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
            <span className="text-sm font-semibold">
              {discussion.author.handle}
            </span>
            {discussion.change_block ? (
              <>
                <span className="text-sm text-muted-foreground">
                  updated the request
                </span>
                <span className="font-mono text-xs font-medium tabular-nums text-foreground">
                  {discussion.change_block.new_head_oid.slice(0, 8)}
                </span>
              </>
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
              <RequestDiscussionReplyTree
                branchStates={replyBranches}
                canQuote={canPostReply}
                expandedReplyIds={expandedReplyIds}
                onLoadChildren={(replyId, before) =>
                  void loadReplyChildren(replyId, before)
                }
                onQuote={(quoted) => setQuoteId(quoted.id)}
                onRetry={(failedReply) =>
                  void postReply(
                    failedReply.body_markdown,
                    failedReply.id,
                    failedReply.reply_to_reply_id,
                  )
                }
                onToggleChildren={(parent) =>
                  void toggleReplyChildren(parent)
                }
                replies={availableReplies}
                visibleReplies={visibleReplies}
              />
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

          <span
            aria-hidden="true"
            className="block h-px"
            ref={readMarkerRef}
          />

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

function ChangeEventMarker() {
  return (
    <div
      aria-hidden="true"
      className="grid size-8 shrink-0 place-items-center rounded-md bg-brand text-brand-foreground shadow-sm"
    >
      <GitCommitHorizontal className="size-4" />
    </div>
  )
}
