import type { RequestSummary } from '@/api/types'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { CheckCircle2, XCircle } from 'lucide-react'
import { useState } from 'react'
import { RequestConfirmDialog } from './request-confirm-dialog'
import { RequestSubmitDialog } from './request-submit-dialog'
import type { RequestActionController } from './use-request-actions'

type Dialog = 'close' | 'merge' | 'submit' | null

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
  const submitLabel = request.author_role === 'Public' ? 'Request review' : 'Mark ready'
  const hasActions = permissions.can_submit ||
    canMerge ||
    permissions.can_close

  if (!hasActions) return null

  return (
    <>
      <div className={cn('flex flex-wrap items-center gap-2', className)}>
        {permissions.can_submit ? (
          <Button disabled={busy} onClick={() => setDialog('submit')} size="sm" type="button">
            <CheckCircle2 />
            {submitLabel}
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

      <RequestSubmitDialog
        onConfirm={() => actions.run({ action: 'submit' })}
        onOpenChange={(open) => setDialog(open ? 'submit' : null)}
        open={dialog === 'submit'}
        pending={actions.pending === 'submit'}
        request={request}
      />
      <RequestConfirmDialog
        confirmLabel="Merge request"
        onConfirm={() => actions.run({ action: 'merge' })}
        onOpenChange={(open) => setDialog(open ? 'merge' : null)}
        open={dialog === 'merge'}
        pending={actions.pending === 'merge'}
        title="Merge this request?"
      >
        <p>This completes “{request.title}” and merges its current head into main.</p>
        <p className="font-mono text-xs">
          {request.head_oid.slice(0, 12)} → main
        </p>
      </RequestConfirmDialog>
      <RequestConfirmDialog
        confirmLabel="Close request"
        destructive
        onConfirm={() => actions.run({ action: 'close' })}
        onOpenChange={(open) => setDialog(open ? 'close' : null)}
        open={dialog === 'close'}
        pending={actions.pending === 'close'}
        title="Close this request?"
      >
        {request.submitted_at_unix === null ? (
          <p>This draft request will be permanently deleted.</p>
        ) : (
          <p>This submitted request will close and remain in request history.</p>
        )}
      </RequestConfirmDialog>
    </>
  )
}
