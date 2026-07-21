import type { RequestChangeBlockFiles, RequestParams, RequestSummary } from '@/api/types'
import type { LoadRequestChangeBlockFilesInput } from '@/api/requests'
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
import type { RequestDiscussionActions } from './request-discussion-store'
import { useRequestDiscussionStore } from './request-discussion-store'
import type {
  RequestActorSummary,
  RequestDiscussion,
  RequestDiscussionPage,
} from './request-discussion-types'
import { RequestChangeBlock } from './request-change-block'

export function RequestDiscussionWorkbench({
  actions,
  actor,
  canResolve,
  contextRail,
  description,
  header,
  initialPage,
  loadChangeBlockFiles,
  onDescriptionSave,
  params,
  permissions,
  repoId,
  request,
  threadActions,
}: {
  actions: RequestDiscussionActions
  actor: RequestActorSummary
  canResolve: (discussion: RequestDiscussion) => boolean
  contextRail: ReactNode
  description: string
  header: (controls: ReactNode) => ReactNode
  initialPage: RequestDiscussionPage
  loadChangeBlockFiles: (
    input: LoadRequestChangeBlockFilesInput,
  ) => Promise<RequestChangeBlockFiles>
  onDescriptionSave: (description: string) => Promise<boolean>
  params: RequestParams
  permissions: {
    canEditDescription: boolean
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

  useEffect(() => {
    const scrollContainer = document.querySelector<HTMLElement>('#main-content')
    if (!scrollContainer) return
    scrollContainer.scrollTop = readRequestDiscussionScroll(store.cacheKey)
    return () => {
      writeRequestDiscussionScroll(store.cacheKey, scrollContainer.scrollTop)
    }
  }, [store.cacheKey])

  return (
    <>
      {header(
        <div className="flex flex-wrap items-center gap-2">
          {store.newActivity ? (
            <Badge variant="info">New activity · order held</Badge>
          ) : null}
          <Button
            className="h-9"
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
        </div>,
      )}
      <div className="grid min-h-0 xl:grid-cols-[minmax(0,1fr)_320px]">
        <div className="min-w-0">
          <RequestDescription
            canEdit={permissions.canEditDescription}
            description={description}
            onSave={onDescriptionSave}
          />

          <section aria-label="Request timeline">
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
                    rootContent={discussion.change_block ? (
                      <RequestChangeBlock
                        block={discussion.change_block}
                        loadFiles={loadChangeBlockFiles}
                        params={params}
                      />
                    ) : undefined}
                  />
                ))}
              </div>
            ) : (
              <div className="border-b border-border px-5 py-14 text-center lg:px-7">
                <MessageSquare className="mx-auto size-5 text-muted-foreground" />
                <h3 className="mt-3 text-sm font-semibold">
                  No timeline activity yet
                </h3>
                <p className="mx-auto mt-1 max-w-md text-sm leading-6 text-muted-foreground">
                  Updates and conversations will appear here in order.
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
                  {store.loadingMore ? 'Loading…' : 'Load earlier activity'}
                </Button>
              </div>
            ) : null}

            {permissions.canOpenDiscussion && request.state !== 'Resolved' && request.state !== 'Withdrawn' ? (
              <div className="border-t border-border px-5 py-5 lg:px-7">
                <RequestDiscussionComposer onSubmit={store.create} />
              </div>
            ) : null}
          </section>
        </div>
        {contextRail}
      </div>
    </>
  )
}
