import type { RequestSummary } from '@/api/types'
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { GitMerge } from 'lucide-react'
import { useState } from 'react'
import {
  fullOid,
  normalizedBody,
  settlementPreviewFor,
  settlementPreviewText,
  shortOid,
} from './request-labels'

export function RequestMergeDialog({
  error,
  onConfirm,
  onOpenChange,
  open,
  pending,
  request,
}: {
  error: string | null
  onConfirm: (body: string | null) => Promise<void>
  onOpenChange: (open: boolean) => void
  open: boolean
  pending: boolean
  request: RequestSummary
}) {
  const [body, setBody] = useState('')
  const currentMainOid = request.mergeability.current_main_oid
  const requestHeadOid = request.mergeability.request_head_oid
  const canConfirm =
    request.mergeability.status === 'Ready' &&
    Boolean(currentMainOid) &&
    Boolean(requestHeadOid)
  const preview = settlementPreviewFor(request.stake_credits, 'Accepted')

  async function confirmMerge() {
    await onConfirm(normalizedBody(body))
  }

  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <div className="flex items-center gap-2 text-base font-semibold leading-6">
            <GitMerge className="size-4 text-muted-foreground" />
            <AlertDialogTitle>Merge request</AlertDialogTitle>
          </div>
          <AlertDialogDescription>
            Are you sure you want to merge this into{' '}
            <span className="font-mono text-foreground">
              {request.target_branch}
            </span>{' '}
            at{' '}
            <span className="font-mono text-foreground">
              {shortOid(currentMainOid)}
            </span>
            ?
          </AlertDialogDescription>
        </AlertDialogHeader>

        <div className="grid gap-3 text-sm leading-5">
          <OidRow label="Current main" value={fullOid(currentMainOid)} />
          <OidRow label="Request head" value={fullOid(requestHeadOid)} />
          <div className="rounded-lg border border-border bg-muted px-3 py-2">
            <div className="text-xs font-medium uppercase text-muted-foreground">
              Settlement preview
            </div>
            <div className="mt-1 font-mono text-xs tabular-nums">
              {settlementPreviewText(preview)}
            </div>
          </div>
          <label className="grid gap-1.5">
            <span className="text-xs font-medium text-muted-foreground">
              Merge note
            </span>
            <textarea
              aria-label="Merge note"
              className={cn(
                'min-h-20 w-full resize-y rounded-lg border border-input',
                'bg-background px-3 py-2 text-sm leading-5 outline-none',
                'placeholder:text-muted-foreground focus-visible:border-ring',
                'focus-visible:ring-3 focus-visible:ring-ring/50',
              )}
              onChange={(event) => setBody(event.target.value)}
              placeholder="Optional note for the request timeline"
              value={body}
            />
          </label>
          {error && (
            <p className="text-sm leading-5 text-destructive">{error}</p>
          )}
        </div>

        <AlertDialogFooter>
          <AlertDialogCancel disabled={pending} size="sm" variant="secondary">
            Cancel
          </AlertDialogCancel>
          <Button
            disabled={pending || !canConfirm}
            onClick={() => void confirmMerge()}
            size="sm"
            type="button"
            variant="success"
          >
            <GitMerge className="size-3.5" />
            <span>{pending ? 'Merging' : 'Merge'}</span>
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}

function OidRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="grid gap-1 rounded-lg border border-border px-3 py-2">
      <div className="text-xs font-medium uppercase text-muted-foreground">
        {label}
      </div>
      <div className="break-all font-mono text-xs leading-5">{value}</div>
    </div>
  )
}
