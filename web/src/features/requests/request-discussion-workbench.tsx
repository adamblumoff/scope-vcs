import type { RequestParams, RequestSummary } from '@/api/types'
import { EmptyState } from '@/components/empty-state'
import { Button } from '@/components/ui/button'
import { CircleAlert, MessageSquare } from 'lucide-react'
import { useEffect, useState } from 'react'
import {
  readRequestDiscussionScroll,
  writeRequestDiscussionScroll,
} from './request-discussion-cache'
import { RequestDiscussionComposer } from './request-discussion-composer'
import type { RequestDiscussionActions } from './request-discussion-store'
import { useRequestDiscussionStore } from './request-discussion-store'
import { RequestDiscussionThread } from './request-discussion-thread'
import type {
  RequestActorSummary,
  RequestDiscussion,
  RequestDiscussionPage,
} from './request-discussion-types'
import type { RequestDiscussionThreadActions } from './use-request-discussion-replies'

export function RequestDiscussionWorkbench({
  actions,
  actor,
  canResolve,
  focusedDiscussionId,
  initialPage,
  params,
  permissions,
  repoId,
  request,
  threadActions,
}: {
  actions: RequestDiscussionActions
  actor: RequestActorSummary
  canResolve: (discussion: RequestDiscussion) => boolean
  focusedDiscussionId?: string
  initialPage: RequestDiscussionPage
  params: RequestParams
  permissions: {
    canOpenDiscussion: boolean
    canReply: boolean
  }
  repoId: string
  request: RequestSummary
  threadActions: RequestDiscussionThreadActions
}) {
  const store = useRequestDiscussionStore({
    actions,
    actor,
    initialPage,
    params,
    repoId,
  })
  const [activeComposer, setActiveComposer] = useState<string | null>(null)

  useEffect(() => {
    const scrollContainer = document.querySelector<HTMLElement>('#main-content')
    if (!scrollContainer) return
    scrollContainer.scrollTop = readRequestDiscussionScroll(store.cacheKey)
    return () => {
      writeRequestDiscussionScroll(store.cacheKey, scrollContainer.scrollTop)
    }
  }, [store.cacheKey])

  useEffect(() => {
    if (!focusedDiscussionId) return
    const frame = requestAnimationFrame(() => {
      document
        .querySelector(`#discussion-${CSS.escape(focusedDiscussionId)}`)
        ?.scrollIntoView({ block: 'start' })
    })
    return () => cancelAnimationFrame(frame)
  }, [focusedDiscussionId, store.cacheKey])

  const canStartDiscussion =
    permissions.canOpenDiscussion &&
    !['Closed', 'Merged'].includes(request.state)

  return (
    <section aria-label="Request discussion">
      {store.error ? (
        <div
          className="flex items-center gap-2 border-b border-border px-5 py-3 text-sm text-destructive lg:px-7"
          role="alert"
        >
          <CircleAlert className="size-4" />
          {store.error}
        </div>
      ) : null}

      {store.collection.nextCursor ? (
        <div className="border-b border-border px-5 py-4 text-center lg:px-7">
          <Button
            disabled={store.loadingMore}
            onClick={() => void store.loadMore()}
            size="sm"
            type="button"
            variant="secondary"
          >
            {store.loadingMore ? 'Loading…' : 'Load earlier discussions'}
          </Button>
        </div>
      ) : null}

      {store.discussions.length > 0 ? (
        <div>
          {store.discussions.map((discussion) => (
            <RequestDiscussionThread
              actions={threadActions}
              actor={actor}
              canReply={permissions.canReply}
              canResolve={canResolve(discussion)}
              composerOpen={activeComposer === discussion.id}
              discussion={discussion}
              key={discussion.id}
              onExpandedChange={store.setExpanded}
              onMarkRead={store.markRead}
              onCloseComposer={() => setActiveComposer(null)}
              onOpenComposer={() => setActiveComposer(discussion.id)}
              onPatch={store.patch}
              onRetryRoot={store.retry}
              onResolve={store.resolve}
              params={params}
            />
          ))}
        </div>
      ) : (
        <EmptyState
          description="Open one to ask a question or leave review notes."
          icon={<MessageSquare />}
          title="No discussions yet"
        />
      )}

      {canStartDiscussion ? (
        <div className="border-t border-border px-5 py-4 lg:px-7">
          <RequestDiscussionComposer onSubmit={store.create} />
        </div>
      ) : null}
    </section>
  )
}
