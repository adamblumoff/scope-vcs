import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { Check, ChevronRight, CircleAlert, Link2, RotateCcw } from 'lucide-react'
import { memo, useEffect } from 'react'
import { compactDiscussionSummary } from './discussion-preview-text'
import { RequestDiscussionAnchor } from './request-discussion-anchor'
import {
  RequestDiscussionActorAvatar,
  RequestDiscussionByline,
} from './request-discussion-byline'
import { RequestReplyComposer } from './request-discussion-composer'
import { RequestDiscussionMarkdown } from './request-discussion-markdown'
import { RequestDiscussionReplyToggle } from './request-discussion-reply-toggle'
import { RequestDiscussionReplyTree } from './request-discussion-reply-tree'
import { requestDiscussionRailColor } from './request-discussion-thread-state'
import type {
  RequestDiscussion,
  RequestDiscussionView,
} from './request-discussion-types'
import { useRequestDiscussionReadMarker } from './use-request-discussion-read-marker'
import {
  useRequestDiscussionReplies,
  type RequestDiscussionThreadActions,
} from './use-request-discussion-replies'

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
    toggleReplies,
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

  const readMarkerRef = useRequestDiscussionReadMarker({
    collapsed,
    contentFullyExposed: previewContentExposed || entireReplyTreeExposed,
    discussion,
    onMarkRead,
  })

  useEffect(() => {
    if (!composerOpen) setQuoteId(null)
  }, [composerOpen, setQuoteId])

  function openComposer() {
    onExpandedChange(discussion.id, true)
    onOpenComposer()
  }

  return (
    <article
      className="request-discussion-thread group/thread grid scroll-mt-32 grid-cols-[2rem_minmax(0,1fr)] gap-x-3 border-t border-border px-5 py-5 first:border-t-0 lg:px-7"
      id={`discussion-${discussion.id}`}
    >
      <div className="relative">
        <RequestDiscussionActorAvatar handle={discussion.author.handle} />
        <span
          aria-hidden="true"
          className={cn(
            'absolute inset-x-0 bottom-0 top-10 mx-auto w-0.5 rounded-full',
            requestDiscussionRailColor(discussion),
          )}
        />
      </div>

      <div className="min-w-0">
        <div className="flex items-start gap-2">
          <RequestDiscussionByline
            author={discussion.author}
            createdAtUnix={discussion.created_at_unix}
          >
            {discussion.unread_count > 0 ? (
              <Badge variant="info">{discussion.unread_count} new</Badge>
            ) : null}
            {discussion.status === 'Resolved' ? (
              <Badge variant="success">
                <Check />
                {discussion.resolved_by
                  ? `Resolved by ${discussion.resolved_by.handle}`
                  : 'Resolved'}
              </Badge>
            ) : null}
            {discussion.pending === 'sending' ? (
              <span className="text-xs text-muted-foreground">Posting…</span>
            ) : null}
            {discussion.pending === 'failed' ? (
              <Badge variant="danger">Failed to post</Badge>
            ) : null}
          </RequestDiscussionByline>
          {!discussion.pending ? (
            <a
              aria-label="Link to discussion"
              className="ml-auto shrink-0 rounded-md p-1 text-muted-foreground opacity-0 transition-opacity hover:bg-muted hover:text-foreground focus-visible:opacity-100 group-hover/thread:opacity-100 max-lg:opacity-100"
              href={`#discussion-${discussion.id}`}
            >
              <Link2 className="size-3.5" />
            </a>
          ) : null}
        </div>

        {discussion.anchor ? (
          <RequestDiscussionAnchor anchor={discussion.anchor} params={params} />
        ) : null}

        {collapsed ? (
          <button
            className="mt-2 flex w-full min-w-0 items-start gap-2 text-left"
            onClick={() => onExpandedChange(discussion.id, true)}
            type="button"
          >
            <ChevronRight className="mt-1 size-3.5 shrink-0 text-muted-foreground" />
            <span className="line-clamp-2 text-sm leading-6 text-muted-foreground">
              {compactDiscussionSummary(discussion.body_markdown)}
            </span>
          </button>
        ) : (
          <RequestDiscussionMarkdown
            className="mt-2 max-w-[68ch]"
            source={discussion.body_markdown}
          />
        )}

        <div className="mt-3 flex flex-wrap items-center gap-2">
          {expandedReplies ||
          discussion.root_reply_count > visibleReplies.length ? (
            <RequestDiscussionReplyToggle
              count={discussion.root_reply_count}
              expanded={expandedReplies}
              keepsPreview
              onToggle={() => void toggleReplies()}
            />
          ) : null}
          {canPostReply ? (
            <Button
              onClick={openComposer}
              size="sm"
              type="button"
              variant="ghost"
            >
              <RotateCcw className="size-3.5" />
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
        </div>

        {!collapsed && visibleReplies.length > 0 ? (
          <div className="mt-2 border-l border-border pb-3 pl-4">
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
              onToggleChildren={(parent) => void toggleReplyChildren(parent)}
              replies={availableReplies}
              visibleReplies={visibleReplies}
            />
          </div>
        ) : null}

        {loadingReplies ? (
          <p className="mt-3 text-xs text-muted-foreground">Loading replies…</p>
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

        <span aria-hidden="true" className="block h-px" ref={readMarkerRef} />

        {!collapsed && canPostReply && composerOpen ? (
          <div className="mt-4 border-t border-border pt-3">
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
                      body: compactDiscussionSummary(quotedReply.body_markdown),
                    }
                  : null
              }
              reopen={discussion.status === 'Resolved'}
            />
          </div>
        ) : null}
      </div>
    </article>
  )
})
