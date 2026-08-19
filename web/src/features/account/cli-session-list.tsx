import type { CliSession } from '@/api/types'
import { DestructiveActionDialog } from '@/components/destructive-action-dialog'
import { Button } from '@/components/ui/button'
import { LoaderCircle, Trash2 } from 'lucide-react'
import { useState } from 'react'

export function CliSessionList({
  formatTime,
  pending,
  revokeSession,
  sessions,
}: {
  formatTime: (value: number) => string
  pending: string | null
  revokeSession: (sessionId: string) => void
  sessions: CliSession[]
}) {
  const [confirmSession, setConfirmSession] = useState<CliSession | null>(null)

  if (sessions.length === 0) {
    return <p className="text-sm leading-5 text-muted-foreground">No active CLI sessions.</p>
  }

  return (
    <>
      <ul className="divide-y divide-border border-y border-border">
        {sessions.map((session) => (
          <li
            className="flex flex-col gap-3 py-3 sm:flex-row sm:items-center sm:justify-between"
            key={session.id}
          >
            <div className="min-w-0">
              <div className="truncate text-sm font-medium leading-5">
                {session.label}
              </div>
              <div className="mt-1 flex flex-wrap gap-x-3 gap-y-1 text-xs leading-4 text-muted-foreground">
                <span>Created {formatTime(session.created_at_unix)}</span>
                {session.last_used_at_unix ? (
                  <span>Used {formatTime(session.last_used_at_unix)}</span>
                ) : null}
                <span>Expires {formatTime(session.expires_at_unix)}</span>
              </div>
            </div>
            <Button
              aria-label={`Revoke ${session.label}`}
              disabled={pending === session.id}
              onClick={() => setConfirmSession(session)}
              size="icon-sm"
              title={`Revoke ${session.label}`}
              type="button"
              variant="destructive"
            >
              {pending === session.id ? (
                <LoaderCircle className="size-3.5 animate-spin" />
              ) : (
                <Trash2 className="size-3.5" />
              )}
            </Button>
          </li>
        ))}
      </ul>
      <DestructiveActionDialog
        confirmLabel="Revoke session"
        description="This CLI session will lose access immediately."
        onConfirm={() => {
          if (confirmSession) {
            revokeSession(confirmSession.id)
            setConfirmSession(null)
          }
        }}
        onOpenChange={(open) => {
          if (!open && !pending) setConfirmSession(null)
        }}
        open={Boolean(confirmSession)}
        pending={Boolean(confirmSession && pending === confirmSession.id)}
        subject={confirmSession?.label ?? ''}
        title="Revoke CLI session?"
      />
    </>
  )
}
