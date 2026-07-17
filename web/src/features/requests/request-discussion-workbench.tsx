import type { RequestParams, RequestSummary } from '@/api/types'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { cn } from '@/lib/utils'
import {
  CircleAlert,
  MessageSquare,
  RefreshCw,
} from 'lucide-react'
import type { ReactNode } from 'react'
import { useEffect } from 'react'
import {
  readRequestDiscussionScroll,
  writeRequestDiscussionScroll,
} from './request-discussion-cache'
import { RequestDescription } from './request-description'
import {
  RequestDiscussionComposer,
} from './request-discussion-composer'
import type {
  RequestDiscussionThreadActions,
} from './request-discussion-thread'
import { RequestDiscussionThread } from './request-discussion-thread'
import type {
  RequestDiscussionActions,
} from './request-discussion-store'
import { useRequestDiscussionStore } from './request-discussion-store'
import type {
  RequestActorSummary,
  RequestDiscussion,
  RequestDiscussionFilter,
  RequestDiscussionPage,
  RequestDiscussionSort,
} from './request-discussion-types'

export function RequestDiscussionWorkbench({
  actions,
  actor,
  canResolve,
  contextRail,
  description,
  filter,
  initialPage,
  onDescriptionSave,
  onQueryChange,
  params,
  permissions,
  repoId,
  request,
  sort,
  threadActions,
}: {
  actions: RequestDiscussionActions
  actor: RequestActorSummary
  canResolve: (discussion: RequestDiscussion) => boolean
  contextRail: ReactNode
  description: string
  filter: RequestDiscussionFilter
  initialPage: RequestDiscussionPage
  onDescriptionSave: (description: string) => Promise<boolean>
  onQueryChange: (query: {
    filter: RequestDiscussionFilter
    sort: RequestDiscussionSort
  }) => void
  params: RequestParams
  permissions: {
    canEditDescription: boolean
    canOpenDiscussion: boolean
    canReply: boolean
  }
  repoId: string
  request: RequestSummary
  sort: RequestDiscussionSort
  threadActions: RequestDiscussionThreadActions
}) {
  const store = useRequestDiscussionStore({
    actions,
    actor,
    filter,
    initialPage,
    params,
    repoId,
    sort,
  })

  useEffect(() => {
    const scrollContainer = document.querySelector<HTMLElement>('#main-content')
    if (!scrollContainer) return
    scrollContainer.scrollTop = readRequestDiscussionScroll(store.cacheKey)
    return () => {
      writeRequestDiscussionScroll(store.cacheKey, scrollContainer.scrollTop)
    }
  }, [store.cacheKey])

  return (
    <div className="grid min-h-0 xl:grid-cols-[minmax(0,1fr)_320px]">
      <div className="min-w-0">
        <RequestDescription
          canEdit={permissions.canEditDescription}
          description={description}
          onSave={onDescriptionSave}
        />

        <section aria-label="Discussion">
          <div className="flex flex-wrap justify-end gap-2 border-b border-border px-5 py-4 lg:px-7">
            <div className="flex flex-wrap items-center gap-2">
              {store.newActivity ? (
                <Badge variant="info">New activity · order held</Badge>
              ) : null}
              <label className="sr-only" htmlFor="discussion-filter">
                Discussion status
              </label>
              <select
                className={selectClass}
                id="discussion-filter"
                onChange={(event) =>
                  onQueryChange({
                    filter: event.target.value as RequestDiscussionFilter,
                    sort,
                  })
                }
                value={filter}
              >
                <option value="Open">Open</option>
                <option value="All">All</option>
              </select>
              <label className="sr-only" htmlFor="discussion-sort">
                Discussion sort
              </label>
              <select
                className={selectClass}
                id="discussion-sort"
                onChange={(event) =>
                  onQueryChange({
                    filter,
                    sort: event.target.value as RequestDiscussionSort,
                  })
                }
                value={sort}
              >
                <option value="Recent">Recently active</option>
                <option value="Newest">Newest</option>
              </select>
              <Button
                disabled={store.refreshing}
                onClick={() => void store.refresh()}
                size="sm"
                type="button"
                variant="secondary"
              >
                <RefreshCw
                  className={cn(
                    'size-3.5',
                    store.refreshing && 'animate-spin',
                  )}
                />
                Refresh
              </Button>
            </div>
          </div>

          {permissions.canOpenDiscussion && request.state !== 'Resolved' && request.state !== 'Withdrawn' ? (
            <div className="border-b border-border px-5 py-5 lg:px-7">
              <RequestDiscussionComposer onSubmit={store.create} />
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
                  discussion={discussion}
                  key={discussion.id}
                  onExpandedChange={store.setExpanded}
                  onMarkRead={store.markRead}
                  onPatch={store.patch}
                  onRetryRoot={store.retry}
                  onSetResolved={store.setResolved}
                  params={params}
                />
              ))}
            </div>
          ) : (
            <div className="border-b border-border px-5 py-14 text-center lg:px-7">
              <MessageSquare className="mx-auto size-5 text-muted-foreground" />
              <h3 className="mt-3 text-sm font-semibold">
                {filter === 'Open'
                  ? 'No open discussions'
                  : 'No discussions yet'}
              </h3>
              <p className="mx-auto mt-1 max-w-md text-sm leading-6 text-muted-foreground">
                {filter === 'Open'
                  ? 'Everything raised so far has been resolved.'
                  : 'Start a focused topic about this request.'}
              </p>
            </div>
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
                {store.loadingMore ? 'Loading…' : 'Load older discussions'}
              </Button>
            </div>
          ) : null}
        </section>
      </div>
      {contextRail}
    </div>
  )
}

const selectClass = cn(
  'h-9 rounded-md border border-input bg-background px-3 text-sm outline-none',
  'focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50',
)
