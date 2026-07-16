import type { RequestEvent } from '@/api/types'
import type { RepoChangeEvent } from '@/api/types.generated'
import { Badge } from '@/components/ui/badge'
import { useRepoChangeSubscription } from '@/features/repo-detail/repo-layout-context'
import { useCallback, useEffect, useRef, useState } from 'react'
import {
  eventKindLabel,
  formatUnixDate,
  requestEventBody,
} from './request-labels'
import {
  MAX_REQUEST_ACTIVITY_EVENTS,
  mergeRequestActivity,
} from './request-activity-model'
import type {
  RequestActivityPage,
  RequestActorSummary,
} from './request-discussion-types'

type ActivityEvent = RequestEvent & { actor: RequestActorSummary }

export function RequestActivity({
  activity,
  loadAfter,
  requestId,
}: {
  activity: RequestActivityPage
  loadAfter: (after: number) => Promise<RequestActivityPage>
  requestId: string
}) {
  return (
    <RequestActivityLive
      activity={activity}
      key={`${requestId}:${activity.through_position}:${activity.events.length}`}
      loadAfter={loadAfter}
      requestId={requestId}
    />
  )
}

function RequestActivityLive({
  activity,
  loadAfter,
  requestId,
}: {
  activity: RequestActivityPage
  loadAfter: (after: number) => Promise<RequestActivityPage>
  requestId: string
}) {
  const [liveActivity, setLiveActivity] =
    useState<RequestActivityPage | null>(null)
  const [updateError, setUpdateError] = useState<string | null>(null)
  const currentRef = useRef(activity)
  const catchUpInFlight = useRef(false)
  const catchUpTarget = useRef(activity.through_position)
  const forceCatchUp = useRef(false)
  const current = liveActivity ?? activity

  const catchUp = useCallback(async () => {
    if (catchUpInFlight.current) return
    catchUpInFlight.current = true
    let failed = false
    try {
      forceCatchUp.current = false
      const page = await loadAfter(currentRef.current.through_position)
      const next = mergeRequestActivity(currentRef.current, page)
      currentRef.current = next
      setLiveActivity(next)
      setUpdateError(null)
    } catch {
      failed = true
      setUpdateError('New request activity could not be loaded.')
    } finally {
      catchUpInFlight.current = false
      if (
        !failed &&
        (
          forceCatchUp.current ||
          currentRef.current.through_position < catchUpTarget.current
        )
      ) {
        void catchUp()
      }
    }
  }, [loadAfter])

  const onRepoChange = useCallback(
    (event: RepoChangeEvent) => {
      if (event.kind === 'Lagged') {
        forceCatchUp.current = true
        void catchUp()
        return
      }
      if (
        typeof event.kind === 'object' &&
        'RequestDiscussionChanged' in event.kind &&
        event.kind.RequestDiscussionChanged.request_id === requestId &&
        event.kind.RequestDiscussionChanged.through_position >
          currentRef.current.through_position
      ) {
        catchUpTarget.current = Math.max(
          catchUpTarget.current,
          event.kind.RequestDiscussionChanged.through_position,
        )
        void catchUp()
      }
    },
    [catchUp, requestId],
  )
  useRepoChangeSubscription(onRepoChange)
  useEffect(() => {
    void catchUp()
  }, [catchUp])

  return (
    <section>
      <div className="flex items-center justify-between gap-3 border-b border-border px-5 py-4 lg:px-7">
        <div>
          <h2 className="text-base font-semibold">Activity</h2>
          <p className="mt-1 text-sm text-muted-foreground">
            Workflow changes and durable request history.
          </p>
        </div>
        <Badge variant="neutral">
          {current.events.length === MAX_REQUEST_ACTIVITY_EVENTS
            ? `Latest ${MAX_REQUEST_ACTIVITY_EVENTS.toLocaleString()} events`
            : `${current.events.length} events`}
        </Badge>
      </div>
      {updateError ? (
        <p
          className="border-b border-border px-5 py-3 text-sm text-destructive lg:px-7"
          role="alert"
        >
          {updateError}
        </p>
      ) : null}
      <div>
        {current.events.map((rawEvent) => {
          const event = rawEvent as ActivityEvent
          const body = requestEventBody(event)
          return (
            <article
              className="grid gap-2 border-b border-border px-5 py-4 lg:px-7"
              key={event.id}
            >
              <div className="flex flex-wrap items-center gap-2">
                <Badge variant="outline">{eventKindLabel(event.kind)}</Badge>
                <span className="font-mono text-xs tabular-nums text-muted-foreground">
                  {formatUnixDate(event.created_at_unix)}
                </span>
                <span className="text-xs text-muted-foreground">
                  {event.actor.handle}
                </span>
              </div>
              {body ? (
                <p className="max-w-4xl whitespace-pre-wrap text-sm leading-6">
                  {body}
                </p>
              ) : null}
            </article>
          )
        })}
      </div>
      {current.events.length === 0 ? (
        <div className="border-b border-border px-5 py-12 text-center text-sm text-muted-foreground lg:px-7">
          No request activity yet.
        </div>
      ) : null}
    </section>
  )
}
