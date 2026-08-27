import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Check,
  ChevronDown,
  ChevronRight,
  CircleAlert,
  Link2,
  Reply,
  RotateCcw,
} from 'lucide-react'
import { memo, useEffect, useRef } from 'react'
import { compactDiscussionSummary } from './discussion-preview-text'
import { RequestDiscussionAnchor } from './request-discussion-anchor'
import {
  RequestDiscussionActorAvatar,
  RequestDiscussionByline,
} from './request-discussion-byline'
import { RequestReplyComposer } from './request-discussion-composer'
import { RequestDiscussionMarkdown } from './request-discussion-markdown'
import { discussionIsCollapsed } from './request-discussion-model'
import { replyTargetFromFragment } from './request-discussion-reply-presentation'
import {
  RequestDiscussionReplyList,
  RequestDiscussionUnreadBoundary,
} from './request-discussion-reply-list'
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
  const collapsed = discussionIsCollapsed(discussion)
  const {
    availableReplies,
    canPostReply,
    hasOlderReplies,
    loadOlderReplies,
    loadReplyTarget,
    loadingReplies,
    olderReplyCount,
    postReply,
    quotedReply,
    replyError,
    setQuoteId,
    unreadContentFullyExposed,
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
    contentFullyExposed: unreadContentFullyExposed,
    discussion,
    onMarkRead,
  })
  const attemptedReplyHashRef = useRef<string | null>(null)

  useEffect(() => {
    let active = true
    function resolveReplyHash() {
      const hash = window.location.hash
      const replyTarget = replyTargetFromFragment(hash)
      if (!replyTarget || replyTarget.discussionId !== discussion.id) return
      const hashHandled = attemptedReplyHashRef.current === hash
      if (collapsed && !hashHandled) {
        onExpandedChange(discussion.id, true)
        return
      }
      const { replyId } = replyTarget
      const target = document.getElementById(`reply-${replyId}`)
      if (target) {
        target.scrollIntoView({ block: 'center' })
        attemptedReplyHashRef.current = hash
        return
      }
      if (hashHandled) return
      attemptedReplyHashRef.current = hash
      void loadReplyTarget(replyId).then(() => {
        if (!active) return
        requestAnimationFrame(resolveReplyHash)
      })
    }

    resolveReplyHash()
    window.addEventListener('hashchange', resolveReplyHash)
    return () => {
      active = false
      window.removeEventListener('hashchange', resolveReplyHash)
    }
  }, [availableReplies, collapsed, discussion.id, loadReplyTarget, onExpandedChange])

  useEffect(() => {
    if (!composerOpen) setQuoteId(null)
  }, [composerOpen, setQuoteId])

  function openComposer() {
    onExpandedChange(discussion.id, true)
    onOpenComposer()
  }

  function toggleCollapsed() {
    if (!collapsed) onCloseComposer()
    onExpandedChange(discussion.id, collapsed)
  }

  async function loadOlderWithoutJump() {
    const firstReplyId = availableReplies.at(0)?.id
    const firstReply = firstReplyId
      ? document.querySelector<HTMLElement>(`#reply-${CSS.escape(firstReplyId)}`)
      : null
    const topBefore = firstReply?.getBoundingClientRect().top
    const loading = loadOlderReplies()
    if (!firstReply || topBefore === undefined) return loading
    await loading
    requestAnimationFrame(() => {
      const topAfter = firstReply.getBoundingClientRect().top
      const scrollContainer = document.querySelector<HTMLElement>('#main-content')
      if (scrollContainer) scrollContainer.scrollTop += topAfter - topBefore
    })
  }

  const rootUnread =
    discussion.unread_count > 0 &&
    discussion.opened_position > discussion.read_through_position

  return (
    <article
      className="request-discussion-thread group/thread relative grid scroll-mt-32 grid-cols-[2rem_minmax(0,1fr)] gap-x-3 px-5 py-5 before:pointer-events-none before:absolute before:inset-x-5 before:top-0 before:border-t before:border-border before:content-[''] first:before:hidden lg:px-7 lg:before:inset-x-7"
      id={`discussion-${discussion.id}`}
    >
      {rootUnread ? (
        <div className="col-span-2">
          <RequestDiscussionUnreadBoundary />
        </div>
      ) : null}

      <div>
        <RequestDiscussionActorAvatar handle={discussion.author.handle} />
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
          <div className="ml-auto flex shrink-0 items-center gap-1">
            {!discussion.pending ? (
              <a
                aria-label="Link to discussion"
                className="shrink-0 rounded-md p-1 text-muted-foreground opacity-0 transition-opacity hover:bg-muted hover:text-foreground focus-visible:opacity-100 group-hover/thread:opacity-100 max-lg:opacity-100"
                href={`#discussion-${discussion.id}`}
              >
                <Link2 className="size-3.5" />
              </a>
            ) : null}
            <button
              aria-expanded={!collapsed}
              aria-label={collapsed ? 'Expand discussion' : 'Collapse discussion'}
              className="shrink-0 rounded-md p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
              onClick={toggleCollapsed}
              title={collapsed ? 'Expand discussion' : 'Collapse discussion'}
              type="button"
            >
              {collapsed ? (
                <ChevronRight className="size-3.5" />
              ) : (
                <ChevronDown className="size-3.5" />
              )}
            </button>
          </div>
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
                <Reply className="size-3.5" />
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
        </div>

        {!collapsed && (availableReplies.length > 0 || hasOlderReplies) ? (
          <div className="mt-2 pb-3">
            {hasOlderReplies ? (
              <button
                className="mb-2 inline-flex items-center gap-1.5 text-xs font-medium text-muted-foreground hover:text-foreground disabled:cursor-wait disabled:opacity-70"
                disabled={loadingReplies}
                onClick={() => void loadOlderWithoutJump()}
                type="button"
              >
                {loadingReplies
                  ? 'Loading…'
                  : `${olderReplyCount} earlier ${olderReplyCount === 1 ? 'reply' : 'replies'}`}
              </button>
            ) : null}
            <RequestDiscussionReplyList
              canReply={canPostReply}
              discussionCreatedAtUnix={discussion.created_at_unix}
              onQuote={(quoted) => {
                setQuoteId(quoted.id)
                openComposer()
              }}
              onRetry={(failedReply) =>
                void postReply(
                  failedReply.body_markdown,
                  failedReply.id,
                  failedReply.reply_to?.id ??
                    failedReply.optimistic_reply_to_reply_id ??
                    null,
                  failedReply.reply_to,
                )
              }
              readThroughPosition={discussion.read_through_position}
              replies={availableReplies}
              showUnreadBoundary={
                discussion.unread_count > 0 &&
                !rootUnread &&
                unreadContentFullyExposed
              }
            />
          </div>
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
