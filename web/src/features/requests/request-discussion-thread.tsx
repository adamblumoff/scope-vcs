import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Check,
  ChevronDown,
  ChevronRight,
  CircleAlert,
  GitCommit,
  Link2,
  MessageSquare,
  RotateCcw,
} from 'lucide-react'
import { memo, useEffect, useEffectEvent, useRef, useState } from 'react'
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
  composerOpen,
  discussion,
  onExpandedChange,
  onMarkRead,
  onOpenComposer,
  onCloseComposer,
  onPatch,
  onRetryRoot,
  onResolve,
  params,
}: {
  actions: RequestDiscussionThreadActions
  actor: { handle: string; id: string }
  canReply: boolean
  canResolve: boolean
  composerOpen: boolean
  discussion: RequestDiscussionView
  onExpandedChange: (discussionId: string, expanded: boolean) => void
  onMarkRead: (discussion: RequestDiscussion) => Promise<void>
  onOpenComposer: () => void
  onCloseComposer: () => void
  onPatch: (discussion: RequestDiscussion) => void
  onRetryRoot: (discussion: RequestDiscussionView) => Promise<boolean>
  onResolve: (discussion: RequestDiscussion) => Promise<void>
  params: { owner: string; repo: string; request_id: string }
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

  useEffect(() => {
    if (!composerOpen) setQuoteId(null)
  }, [composerOpen, setQuoteId])

  function openComposer() {
    onExpandedChange(discussion.id, true)
    onOpenComposer()
  }

  return (
    <article
      className="request-discussion-thread scroll-mt-32 border-t border-border px-5 py-5 first:border-t-0 lg:px-7"
      id={`discussion-${discussion.id}`}
    >
      <div className="flex min-w-0 items-start gap-3">
        <RequestDiscussionActorAvatar handle={discussion.author.handle} />
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
            <span className="text-sm font-semibold">
              {discussion.author.handle}
            </span>
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

          {discussion.anchor ? (
            <RequestDiscussionAnchor
              anchor={discussion.anchor}
              params={params}
            />
          ) : null}

          {collapsed ? (
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
            <RequestDiscussionMarkdown
              className="mt-2"
              source={discussion.body_markdown}
            />
          )}

          <div className="mt-3 flex flex-wrap items-center gap-2">
            {discussion.reply_count > 0 ? (
              <Button
                onClick={() => void expandReplies()}
                size="sm"
                type="button"
                variant="ghost"
              >
                <MessageSquare className="size-3.5" />
                {`${discussion.reply_count} ${discussion.reply_count === 1 ? 'reply' : 'replies'}`}
                <ChevronDown className="size-3.5" />
              </Button>
            ) : null}
            {canPostReply ? (
              <Button
                onClick={openComposer}
                size="sm"
                type="button"
                variant="ghost"
              >
                {discussion.status === 'Resolved' ? (
                  <RotateCcw className="size-3.5" />
                ) : (
                  <MessageSquare className="size-3.5" />
                )}
                {discussion.status === 'Resolved' ? 'Reopen and reply' : 'Reply'}
              </Button>
            ) : null}
            {canResolve && discussion.status === 'Open' && !discussion.pending ? (
              <Button
                onClick={() => void onResolve(discussion)}
                size="sm"
                type="button"
                variant="ghost"
              >
                <Check className="size-3.5" />
                Resolve
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
                onQuote={(quoted) => {
                  setQuoteId(quoted.id)
                  openComposer()
                }}
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

          {!collapsed && canPostReply && composerOpen ? (
            <div className="mt-4">
              <RequestReplyComposer
                onCancel={() => {
                  setQuoteId(null)
                  onCloseComposer()
                }}
                onCancelQuote={() => setQuoteId(null)}
                onSubmit={async (body) => {
                  const posted = await postReply(body)
                  if (posted) onCloseComposer()
                  return posted
                }}
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

function RequestDiscussionAnchor({
  anchor,
  params,
}: {
  anchor: NonNullable<RequestDiscussion['anchor']>
  params: { owner: string; repo: string; request_id: string }
}) {
  if (!anchor) return null
  const search = new URLSearchParams({ revision: anchor.revision_id })
  if (anchor.commit_oid) search.set('commit', anchor.commit_oid)
  if (anchor.path) search.set('path', anchor.path)
  const href = `/${encodeURIComponent(params.owner)}/${encodeURIComponent(params.repo)}/requests/${encodeURIComponent(params.request_id)}/changes?${search}`
  return (
    <a
      className="mt-2 inline-flex max-w-full items-center gap-2 font-mono text-xs text-muted-foreground hover:text-brand"
      href={href}
    >
      <GitCommit className="size-3.5 shrink-0" />
      <span className="truncate">
        Revision {anchor.revision_id.slice(0, 10)}
        {anchor.commit_oid ? ` · ${anchor.commit_oid.slice(0, 10)}` : ''}
        {anchor.path ? ` · ${anchor.path.replace(/^\/+/, '')}` : ''}
      </span>
    </a>
  )
}
