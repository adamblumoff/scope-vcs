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
              The assessment is final and completes the request.
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
                    {outcomeText(value)}
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
              aria-invalid={rejectionNeedsReason}
              className="min-h-28 w-full resize-y rounded-lg border border-input bg-secondary px-3 py-2 text-sm leading-6 outline-none focus-visible:border-ring focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring aria-invalid:border-destructive"
              id={bodyId}
              onChange={(event) => setBody(event.target.value)}
              value={body}
            />
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

function outcomeText(outcome: RequestWorkflowAssessmentOutcome) {
  if (outcome === 'Accepted') return 'Complete the request as accepted.'
  if (outcome === 'Neutral') return 'Complete without a positive or negative judgment.'
  return 'Complete as rejected; a written reason is required.'
}
