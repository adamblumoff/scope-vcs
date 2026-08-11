import type { RequestParams, RequestSummary } from '@/api/types'
import { EmptyState } from '@/components/empty-state'
import { Button } from '@/components/ui/button'
import { CircleAlert, MessageSquare } from 'lucide-react'
import { useEffect, useState } from 'react'
import {
  readRequestDiscussionScroll,
  writeRequestDiscussionScroll,
} from './request-discussion-cache'
import {
  RequestDiscussionComposer,
} from './request-discussion-composer'
import { RequestDiscussionThread } from './request-discussion-thread'
import type {
  RequestDiscussionThreadActions,
} from './use-request-discussion-replies'
import type { RequestDiscussionActions } from './request-discussion-store'
import { useRequestDiscussionStore } from './request-discussion-store'
import type {
  RequestActorSummary,
  RequestDiscussion,
  RequestDiscussionPage,
} from './request-discussion-types'

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
  const [activeComposer, setActiveComposer] = useState<string | 'new' | null>(null)

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

  const canStartDiscussion = permissions.canOpenDiscussion &&
    !['Closed', 'Merged'].includes(request.state)

  return (
    <section aria-label="Request discussion">
      {canStartDiscussion ? (
        <div className="flex justify-end px-5 py-3 lg:px-7">
          <Button
            onClick={() => setActiveComposer('new')}
            size="sm"
            type="button"
            variant="secondary"
          >
            Start discussion
          </Button>
        </div>
      ) : null}
      {activeComposer === 'new' ? (
        <div className="border-b border-border px-5 py-5 lg:px-7">
          <RequestDiscussionComposer
            onCancel={() => setActiveComposer(null)}
            onSubmit={async (body) => {
              const posted = await store.create(body)
              if (posted) setActiveComposer(null)
              return posted
            }}
          />
        </div>
      ) : null}

      {store.error ? (
        <div
          className="flex items-center gap-2 border-b border-border px-5 py-3 text-sm text-destructive lg:px-7"
          role="alert"
        >
          <CircleAlert className="size-4" />
          {store.error}
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
          description="Updates and conversations will appear here in order."
          icon={<MessageSquare />}
          title="No timeline activity yet"
        />
      )}

      {store.collection.nextCursor ? (
        <div className="border-t border-border px-5 py-5 text-center lg:px-7">
          <Button
            disabled={store.loadingMore}
            onClick={() => void store.loadMore()}
            size="sm"
            type="button"
            variant="secondary"
          >
            {store.loadingMore ? 'Loading…' : 'Load earlier activity'}
          </Button>
        </div>
      ) : null}
    </section>
  )
}
