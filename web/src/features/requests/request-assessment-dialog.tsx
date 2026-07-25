import type {
  RequestSummary,
  RequestWorkflowAssessmentOutcome,
} from '@/api/types'
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
import { LoaderCircle } from 'lucide-react'
import type { FormEvent } from 'react'
import { useId, useState } from 'react'

const OUTCOMES = ['Accepted', 'Neutral', 'Rejected'] as const

export function RequestAssessmentDialog({
  onConfirm,
  onOpenChange,
  open,
  pending,
  request,
}: {
  onConfirm: (
    outcome: RequestWorkflowAssessmentOutcome,
    bodyMarkdown: string | null,
  ) => Promise<boolean>
  onOpenChange: (open: boolean) => void
  open: boolean
  pending: boolean
  request: RequestSummary
}) {
  const bodyId = useId()
  const [outcome, setOutcome] = useState<RequestWorkflowAssessmentOutcome>('Accepted')
  const [body, setBody] = useState('')
  const rejectionNeedsReason = outcome === 'Rejected' && !body.trim()
  const preview = request.assessment_previews.find((item) => item.outcome === outcome)

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (rejectionNeedsReason || pending) return
    const note = body.trim() || null
    if (await onConfirm(outcome, note)) onOpenChange(false)
  }

  return (
    <AlertDialog
      onOpenChange={(nextOpen) => {
        if (!pending) onOpenChange(nextOpen)
      }}
      open={open}
    >
      <AlertDialogContent asChild>
        <form onSubmit={(event) => void submit(event)}>
          <AlertDialogHeader>
            <AlertDialogTitle>Assess request</AlertDialogTitle>
            <AlertDialogDescription>
              The assessment is final and completes the request. Settlement commits with it.
            </AlertDialogDescription>
          </AlertDialogHeader>

          <fieldset className="grid border-y border-border">
            <legend className="sr-only">Assessment outcome</legend>
            {OUTCOMES.map((value) => (
              <label
                className={cn(
                  'grid cursor-pointer grid-cols-[auto_minmax(0,1fr)] gap-x-3 border-b border-border px-3 py-3 last:border-b-0',
                  value === outcome && 'bg-muted/60',
                )}
                key={value}
              >
                <input
                  checked={value === outcome}
                  className="mt-1 size-4 accent-primary"
                  name="assessment-outcome"
                  onChange={() => setOutcome(value)}
                  type="radio"
                  value={value}
                />
                <span>
                  <span className="block text-sm font-semibold">{value}</span>
                  <span className="mt-0.5 block text-xs leading-5 text-muted-foreground">
                    {previewText(request, value)}
                  </span>
                </span>
              </label>
            ))}
          </fieldset>

          <div className="grid gap-2">
            <label className="text-sm font-medium" htmlFor={bodyId}>
              {outcome === 'Rejected' ? 'Rejection reason' : 'Assessment note · optional'}
            </label>
            <textarea
              aria-describedby={`${bodyId}-preview`}
              aria-invalid={rejectionNeedsReason}
              className="min-h-28 w-full resize-y rounded-lg border border-input bg-secondary px-3 py-2 text-sm leading-6 outline-none focus-visible:border-ring focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring aria-invalid:border-destructive"
              id={bodyId}
              onChange={(event) => setBody(event.target.value)}
              value={body}
            />
            <p className="text-xs text-muted-foreground" id={`${bodyId}-preview`}>
              Exact settlement: {preview ? settlementText(preview) : 'unavailable'}.
            </p>
            {rejectionNeedsReason ? (
              <p className="text-sm text-destructive" role="alert">
                Rejected assessments require a written reason.
              </p>
            ) : null}
          </div>

          <AlertDialogFooter>
            <AlertDialogCancel disabled={pending} size="sm">
              Cancel
            </AlertDialogCancel>
            <Button disabled={rejectionNeedsReason || pending} size="sm" type="submit">
              {pending ? <LoaderCircle className="animate-spin" /> : null}
              Complete as {outcome}
            </Button>
          </AlertDialogFooter>
        </form>
      </AlertDialogContent>
    </AlertDialog>
  )
}

function previewText(
  request: RequestSummary,
  outcome: RequestWorkflowAssessmentOutcome,
) {
  const preview = request.assessment_previews.find((item) => item.outcome === outcome)
  return preview ? settlementText(preview) : 'Settlement preview unavailable'
}

function settlementText(preview: RequestSummary['assessment_previews'][number]) {
  if (preview.outcome === 'Accepted') {
    return `${preview.refunded_credits} returned + ${preview.reward_credits} reward`
  }
  if (preview.outcome === 'Neutral') {
    return `${preview.refunded_credits} returned · no reward`
  }
  return `${preview.burned_credits} burned`
}
