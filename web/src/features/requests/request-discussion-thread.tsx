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
  Reply,
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
  directDiscussionReplies,
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

type ReplyBranchState = {
  error: string | null
  knownChildCount: number
  loaded: boolean
  loading: boolean
  nextBeforePosition: number | null
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
  const [rootRepliesLoaded, setRootRepliesLoaded] = useState(false)
  const [loadingReplies, setLoadingReplies] = useState(false)
  const [nextBeforePosition, setNextBeforePosition] = useState<number | null>(
    null,
  )
  const [replies, setReplies] = useState<RequestDiscussionReplyView[]>([])
  const [expandedReplyIds, setExpandedReplyIds] = useState<Set<string>>(
    new Set(),
  )
  const [replyBranches, setReplyBranches] = useState<
    Map<string, ReplyBranchState>
  >(new Map())
  const [replyError, setReplyError] = useState<string | null>(null)
  const [quoteId, setQuoteId] = useState<string | null>(null)
  const [readMarkerVisible, setReadMarkerVisible] = useState(false)
  const readMarkerRef = useRef<HTMLSpanElement>(null)
  const markReadAttemptRef = useRef<number | null>(null)
  const collapsed =
    discussion.status === 'Resolved' &&
    Boolean(discussion.initiallyResolved) &&
    !discussion.expanded

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

  async function expandReplies() {
    onExpandedChange(discussion.id, true)
    if (expandedReplies && (rootRepliesLoaded || loadingReplies)) return
    setExpandedReplies(true)
    if (rootRepliesLoaded || discussion.reply_count === 0) return
    setLoadingReplies(true)
    setReplyError(null)
    try {
      const page = await actions.loadReplies({
        ...params,
        discussion_id: discussion.id,
      })
      setReplies((current) => mergeDiscussionReplies(
        current,
        [...discussion.latest_replies, ...page.replies],
      ))
      setNextBeforePosition(page.next_before_position)
      setRootRepliesLoaded(true)
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
      setReplies((current) => mergeDiscussionReplies(current, page.replies))
      setNextBeforePosition(page.next_before_position)
    } catch (error) {
      setReplyError(messageFor(error, 'Older replies could not be loaded.'))
    } finally {
      setLoadingReplies(false)
    }
  }

  function patchReplyBranch(
    replyId: string,
    patch: Partial<ReplyBranchState>,
  ) {
    setReplyBranches((current) => {
      const next = new Map(current)
      next.set(replyId, {
        error: null,
        knownChildCount: 0,
        loaded: false,
        loading: false,
        nextBeforePosition: null,
        ...current.get(replyId),
        ...patch,
      })
      return next
    })
  }

  async function loadReplyChildren(
    parentReplyId: string,
    before?: number,
  ) {
    patchReplyBranch(parentReplyId, { error: null, loading: true })
    try {
      const page = await actions.loadReplies({
        ...params,
        before,
        discussion_id: discussion.id,
        parent_reply_id: parentReplyId,
      })
      setReplies((current) => mergeDiscussionReplies(current, page.replies))
      const parent = availableReplies.find(
        (reply) => reply.id === parentReplyId,
      )
      const loadedChildCount = directDiscussionReplies(
        mergeDiscussionReplies(availableReplies, page.replies),
        parentReplyId,
      ).length
      const knownChildCount = Math.max(
        parent?.child_reply_count ?? 0,
        loadedChildCount,
      )
      const nextBranch = {
        error: null,
        knownChildCount,
        loaded: true,
        loading: false,
        nextBeforePosition: page.next_before_position,
      }
      setReplyBranches((current) =>
        new Map(current).set(parentReplyId, nextBranch),
      )
    } catch (error) {
      patchReplyBranch(parentReplyId, {
        error: messageFor(error, 'Replies could not be loaded.'),
        loading: false,
      })
    }
  }

  async function toggleReplyChildren(reply: RequestDiscussionReplyView) {
    if (expandedReplyIds.has(reply.id)) {
      setExpandedReplyIds((current) => {
        const next = new Set(current)
        next.delete(reply.id)
        return next
      })
      return
    }
    setExpandedReplyIds((current) => new Set(current).add(reply.id))
    const branch = replyBranches.get(reply.id)
    if (
      branch?.loaded &&
      branch.knownChildCount >= reply.child_reply_count
    ) return
    await loadReplyChildren(reply.id)
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
    setReplies((current) => {
      const merged = mergeDiscussionReplies(
        current,
        discussion.latest_replies,
      )
      const alreadyPresent = merged.some((reply) => reply.id === clientReplyId)
      const withParentCount = merged.map((reply) =>
        !alreadyPresent && reply.id === replyToReplyId
          ? { ...reply, child_reply_count: reply.child_reply_count + 1 }
          : reply,
      )
      return upsertDiscussionReply(withParentCount, [], optimistic)
    })
    if (replyToReplyId) {
      setExpandedReplyIds((current) => new Set(current).add(replyToReplyId))
    }
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
      setQuoteId(null)
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
  const visibleReplies: RequestDiscussionReplyView[] = expandedReplies
    ? directDiscussionReplies(availableReplies, null)
    : directDiscussionReplies(discussion.latest_replies, null)
  const canPostReply =
    canReply &&
    (
      discussion.status === 'Dormant' ||
      discussion.status === 'Open' ||
      canResolve
    )
  const entireReplyTreeExposed = replyTreeFullyExposed({
    branches: replyBranches,
    expandedReplyIds,
    nextBeforePosition,
    replies: availableReplies,
    replyCount: discussion.reply_count,
    rootRepliesLoaded:
      rootRepliesLoaded ||
      discussion.latest_replies.length >= discussion.reply_count,
  })
  const previewContentExposed =
    discussion.reply_count <= discussion.latest_replies.length &&
    discussion.latest_replies.every(
      (reply) => reply.reply_to_reply_id === null,
    )
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
          <ActorAvatar handle={discussion.author.handle} />
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
              {visibleReplies.map((reply) => (
                <DiscussionReply
                  branchStates={replyBranches}
                  canQuote={canPostReply}
                  expandedReplyIds={expandedReplyIds}
                  key={reply.id}
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
                  reply={reply}
                  replies={availableReplies}
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

function DiscussionReply({
  branchStates,
  canQuote,
  expandedReplyIds,
  onLoadChildren,
  onQuote,
  onRetry,
  onToggleChildren,
  reply,
  replies,
}: {
  branchStates: Map<string, ReplyBranchState>
  canQuote: boolean
  expandedReplyIds: Set<string>
  onLoadChildren: (replyId: string, before?: number) => void
  onQuote: (reply: RequestDiscussionReplyView) => void
  onRetry: (reply: RequestDiscussionReplyView) => void
  onToggleChildren: (reply: RequestDiscussionReplyView) => void
  reply: RequestDiscussionReplyView
  replies: RequestDiscussionReplyView[]
}) {
  const children = directDiscussionReplies(replies, reply.id)
  const childCount = Math.max(reply.child_reply_count, children.length)
  const expanded = expandedReplyIds.has(reply.id)
  const branch = branchStates.get(reply.id)

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
            aria-expanded={expanded}
            className="text-xs font-medium text-brand hover:text-foreground"
            onClick={() => onToggleChildren(reply)}
            type="button"
          >
            {expanded
              ? `− Hide ${childCount === 1 ? 'reply' : 'replies'}`
              : `+ ${childCount} ${childCount === 1 ? 'reply' : 'replies'}`}
          </button>
        ) : null}
      </div>

      {expanded ? (
        <div className="mt-3 border-l border-border pl-4">
          {branch?.nextBeforePosition !== null &&
          branch?.nextBeforePosition !== undefined ? (
            <button
              className="mb-3 text-xs font-medium text-muted-foreground hover:text-foreground"
              disabled={branch.loading}
              onClick={() =>
                onLoadChildren(reply.id, branch.nextBeforePosition as number)
              }
              type="button"
            >
              {branch.loading ? 'Loading…' : 'Load older replies'}
            </button>
          ) : null}
          {children.map((child) => (
            <DiscussionReply
              branchStates={branchStates}
              canQuote={canQuote}
              expandedReplyIds={expandedReplyIds}
              key={child.id}
              onLoadChildren={onLoadChildren}
              onQuote={onQuote}
              onRetry={onRetry}
              onToggleChildren={onToggleChildren}
              reply={child}
              replies={replies}
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
                  onLoadChildren(
                    reply.id,
                    branch.nextBeforePosition ?? undefined,
                  )
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
    child_reply_count: 0,
    can_reply: false,
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

function replyTreeFullyExposed({
  branches,
  expandedReplyIds,
  nextBeforePosition,
  replies,
  replyCount,
  rootRepliesLoaded,
}: {
  branches: Map<string, ReplyBranchState>
  expandedReplyIds: Set<string>
  nextBeforePosition: number | null
  replies: RequestDiscussionReplyView[]
  replyCount: number
  rootRepliesLoaded: boolean
}) {
  return (
    rootRepliesLoaded &&
    nextBeforePosition === null &&
    replies.length >= replyCount &&
    replies.every((reply) =>
      reply.child_reply_count === 0 ||
      (
        expandedReplyIds.has(reply.id) &&
        branches.get(reply.id)?.loaded === true &&
        branches.get(reply.id)?.nextBeforePosition === null
      ),
    )
  )
}
