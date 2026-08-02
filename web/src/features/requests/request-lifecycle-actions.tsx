import type { RequestSummary } from '@/api/types'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { CheckCircle2, RotateCcw, XCircle } from 'lucide-react'
import { useState } from 'react'
import { RequestConfirmDialog } from './request-confirm-dialog'
import { RequestReadyDialog } from './request-ready-dialog'
import type { RequestActionController } from './use-request-actions'

type Dialog = 'close' | 'merge' | 'ready' | 'working' | null

export function RequestLifecycleActions({
  actions,
  className,
  request,
}: {
  actions: RequestActionController
  className?: string
  request: RequestSummary
}) {
  const [dialog, setDialog] = useState<Dialog>(null)
  const busy = actions.pending !== null
  const permissions = request.permissions
  const canMerge = permissions.can_merge && request.mergeability.status === 'Ready'
  const hasActions = permissions.can_mark_ready ||
    permissions.can_return_to_working ||
    canMerge ||
    permissions.can_close

  if (!hasActions) return null

  return (
    <>
      <div className={cn('flex flex-wrap items-center gap-2', className)}>
        {permissions.can_mark_ready ? (
          <Button disabled={busy} onClick={() => setDialog('ready')} size="sm" type="button">
            <CheckCircle2 />
            Ready for review
          </Button>
        ) : null}
        {permissions.can_return_to_working ? (
          <Button disabled={busy} onClick={() => setDialog('working')} size="sm" type="button" variant="secondary">
            <RotateCcw />
            Return to Working
          </Button>
        ) : null}
        {canMerge ? (
          <Button disabled={busy} onClick={() => setDialog('merge')} size="sm" type="button" variant="success">
            Merge
          </Button>
        ) : null}
        {permissions.can_close ? (
          <Button disabled={busy} onClick={() => setDialog('close')} size="sm" type="button" variant="destructive">
            <XCircle />
            Close
          </Button>
        ) : null}
      </div>

      <RequestReadyDialog
        onConfirm={() => actions.run({ action: 'ready' })}
        onOpenChange={(open) => setDialog(open ? 'ready' : null)}
        open={dialog === 'ready'}
        pending={actions.pending === 'ready'}
        request={request}
      />
      <RequestConfirmDialog
        confirmLabel="Return to Working"
        onConfirm={() => actions.run({ action: 'working' })}
        onOpenChange={(open) => setDialog(open ? 'working' : null)}
        open={dialog === 'working'}
        pending={actions.pending === 'working'}
        title="Return this request to Working?"
      >
        <p>The author must mark the current package Ready again before review can continue.</p>
      </RequestConfirmDialog>
      <RequestConfirmDialog
        confirmLabel="Merge request"
        onConfirm={() => actions.run({ action: 'merge' })}
        onOpenChange={(open) => setDialog(open ? 'merge' : null)}
        open={dialog === 'merge'}
        pending={actions.pending === 'merge'}
        title="Merge this request?"
      >
        <p>This completes the request and merges it into main.</p>
      </RequestConfirmDialog>
      <RequestConfirmDialog
        confirmLabel="Close request"
        destructive
        onConfirm={() => actions.run({ action: 'close' })}
        onOpenChange={(open) => setDialog(open ? 'close' : null)}
        open={dialog === 'close'}
        pending={actions.pending === 'close'}
        title="Close this Working request?"
      >
        {request.first_ready_at_unix === null ? (
          <p>This never-published request will be deleted and will not enter public history.</p>
        ) : (
          <p>This published request will become Completed and remain in public history.</p>
        )}
      </RequestConfirmDialog>
    </>
  )
}
