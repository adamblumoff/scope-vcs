import type { RequestSummary } from '@/api/types'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { CheckCircle2, LoaderCircle, Pause, Play, RotateCcw, XCircle } from 'lucide-react'
import { useState } from 'react'
import { RequestAssessmentDialog } from './request-assessment-dialog'
import { RequestConfirmDialog } from './request-confirm-dialog'
import { RequestReadyDialog } from './request-ready-dialog'
import type { RequestActionController } from './use-request-actions'

type Dialog = 'assess' | 'close' | 'merge' | 'ready' | 'request_changes' | 'working' | null

export function RequestLifecycleActions({
  actions,
  balance,
  className,
  request,
}: {
  actions: RequestActionController
  balance: number | null
  className?: string
  request: RequestSummary
}) {
  const [dialog, setDialog] = useState<Dialog>(null)
  const busy = actions.pending !== null
  const permissions = request.permissions
  const canMerge = permissions.can_merge && request.mergeability.status === 'Ready'
  const hasActions = permissions.can_mark_ready ||
    permissions.can_return_to_working ||
    permissions.can_hold ||
    permissions.can_request_changes ||
    permissions.can_assess ||
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
        {permissions.can_hold ? (
          <Button
            disabled={busy}
            onClick={() => void actions.run({
              action: request.held_at_unix === null ? 'hold' : 'release_hold',
            })}
            size="sm"
            type="button"
            variant="secondary"
          >
            {actions.pending === 'hold' || actions.pending === 'release_hold' ? (
              <LoaderCircle className="animate-spin" />
            ) : request.held_at_unix === null ? <Pause /> : <Play />}
            {request.held_at_unix === null ? 'Hold' : 'Release hold'}
          </Button>
        ) : null}
        {permissions.can_request_changes ? (
          <Button disabled={busy} onClick={() => setDialog('request_changes')} size="sm" type="button" variant="secondary">
            Request changes
          </Button>
        ) : null}
        {permissions.can_assess ? (
          <Button disabled={busy} onClick={() => setDialog('assess')} size="sm" type="button" variant="secondary">
            Assess…
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
        balance={balance}
        onConfirm={(stake_credits) => actions.run({ action: 'ready', stake_credits })}
        onOpenChange={(open) => setDialog(open ? 'ready' : null)}
        open={dialog === 'ready'}
        pending={actions.pending === 'ready'}
        request={request}
      />
      <RequestAssessmentDialog
        onConfirm={(outcome, body_markdown) => actions.run({
          action: 'assess',
          body_markdown,
          outcome,
        })}
        onOpenChange={(open) => setDialog(open ? 'assess' : null)}
        open={dialog === 'assess'}
        pending={actions.pending === 'assess'}
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
        <p>{request.current_stake_credits} credits will be refunded atomically.</p>
        <p>The author must mark the current package Ready again before review can continue.</p>
      </RequestConfirmDialog>
      <RequestConfirmDialog
        confirmLabel="Request changes"
        onConfirm={() => actions.run({ action: 'request_changes' })}
        onOpenChange={(open) => setDialog(open ? 'request_changes' : null)}
        open={dialog === 'request_changes'}
        pending={actions.pending === 'request_changes'}
        title="Request changes from the author?"
      >
        <p>No written explanation is required.</p>
        <p>{request.current_stake_credits} credits will be refunded and any hold will be cleared in the same transaction.</p>
      </RequestConfirmDialog>
      <RequestConfirmDialog
        confirmLabel="Merge request"
        onConfirm={() => actions.run({ action: 'merge' })}
        onOpenChange={(open) => setDialog(open ? 'merge' : null)}
        open={dialog === 'merge'}
        pending={actions.pending === 'merge'}
        title="Merge this request?"
      >
        {request.state === 'ReadyForReview' ? (
          <p>This completes as Accepted. {acceptedSettlement(request)}</p>
        ) : (
          <p>This request is already Accepted. Merging it has no second credit effect.</p>
        )}
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
          <p>This published request will become Completed and remain in public history without an assessment.</p>
        )}
      </RequestConfirmDialog>
    </>
  )
}

function acceptedSettlement(request: RequestSummary) {
  const preview = request.assessment_previews.find((item) => item.outcome === 'Accepted')
  return preview
    ? `${preview.refunded_credits} credits return and ${preview.reward_credits} reward credits are added.`
    : 'Settlement preview is unavailable.'
}
