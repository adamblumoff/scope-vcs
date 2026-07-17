import type { RequestEvent } from '@/api/types'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import * as Dialog from '@radix-ui/react-dialog'
import { History, TriangleAlert, X } from 'lucide-react'
import {
  eventKindLabel,
  formatUnixDate,
  requestEventBody,
} from './request-labels'
import type {
  RequestActivityPage,
  RequestActorSummary,
} from './request-discussion-types'

type ActivityEvent = RequestEvent & { actor: RequestActorSummary }

export function RequestActivityDrawer({
  activity,
  error,
  loading,
  load,
  onOpenChange,
  open,
}: {
  activity: RequestActivityPage | null
  error: string | null
  loading: boolean
  load: () => void
  onOpenChange: (open: boolean) => void
  open: boolean
}) {
  const events = activity
    ? ([...activity.events].reverse() as ActivityEvent[])
    : []

  return (
    <Dialog.Root onOpenChange={onOpenChange} open={open}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-50 bg-background/80 backdrop-blur-sm" />
        <Dialog.Content
          aria-describedby="request-history-description"
          className="fixed inset-y-0 right-0 z-50 flex w-[520px] max-w-[90vw] flex-col border-l border-[var(--border-strong)] bg-background shadow-[var(--shadow-pop)] outline-none"
        >
          <div className="flex min-h-16 items-start gap-3 border-b border-border px-5 py-4">
            <History className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
            <div className="min-w-0 flex-1">
              <Dialog.Title className="text-sm font-semibold">
                Request history
              </Dialog.Title>
              <Dialog.Description
                className="mt-1 text-xs leading-5 text-muted-foreground"
                id="request-history-description"
              >
                Durable workflow changes, newest first.
              </Dialog.Description>
            </div>
            <Dialog.Close asChild>
              <Button
                aria-label="Close request history"
                size="icon-xs"
                type="button"
                variant="ghost"
              >
                <X className="size-3.5" />
              </Button>
            </Dialog.Close>
          </div>

          <div className="min-h-0 flex-1 overflow-y-auto">
            {loading ? (
              <p className="px-5 py-8 text-sm text-muted-foreground">
                Loading request history…
              </p>
            ) : error ? (
              <div
                className="flex items-start gap-3 px-5 py-8 text-sm"
                role="alert"
              >
                <TriangleAlert className="mt-0.5 size-4 shrink-0 text-destructive" />
                <div className="grid gap-3">
                  <p className="text-destructive">{error}</p>
                  <div>
                    <Button
                      onClick={load}
                      size="sm"
                      type="button"
                      variant="secondary"
                    >
                      Retry
                    </Button>
                  </div>
                </div>
              </div>
            ) : events.length > 0 ? (
              events.map((event) => {
                const body = requestEventBody(event)
                return (
                  <article
                    className="grid gap-2 border-b border-border px-5 py-4"
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
                      <p className="whitespace-pre-wrap text-sm leading-6">
                        {body}
                      </p>
                    ) : null}
                  </article>
                )
              })
            ) : (
              <p className="px-5 py-8 text-sm text-muted-foreground">
                No request history yet.
              </p>
            )}
          </div>
          {activity?.events.length === 50 ? (
            <p className="border-t border-border px-5 py-3 text-xs text-muted-foreground">
              Showing the latest 50 events.
            </p>
          ) : null}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  )
}
