import type { RequestDetail } from '@/api/types'
import { Badge } from '@/components/ui/badge'
import {
  eventKindLabel,
  formatUnixDate,
  requestEventBody,
} from './request-labels'

export function RequestTimeline({ detail }: { detail: RequestDetail }) {
  return (
    <section className="mt-8">
      <div className="flex items-center justify-between gap-3">
        <h2 className="text-balance text-lg font-semibold leading-7">Activity</h2>
        <Badge variant="neutral">{detail.events.length} events</Badge>
      </div>
      <div className="mt-2 divide-y divide-border border-t border-border">
        {detail.events.map((event) => {
          const body = requestEventBody(event)
          return (
            <article className="grid gap-2 py-4" key={event.id}>
              <div className="flex flex-wrap items-center gap-2">
                <Badge variant="outline">{eventKindLabel(event.kind)}</Badge>
                <span className="font-mono text-xs tabular-nums text-muted-foreground">
                  {formatUnixDate(event.created_at_unix)}
                </span>
                <span className="break-all font-mono text-xs text-muted-foreground">
                  Actor {event.actor_user_id}
                </span>
              </div>
              {body && (
                <p className="text-pretty whitespace-pre-wrap text-sm leading-6">
                  {body}
                </p>
              )}
            </article>
          )
        })}
      </div>
    </section>
  )
}
