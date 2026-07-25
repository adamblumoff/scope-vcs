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
import { Input } from '@/components/ui/input'
import { LoaderCircle } from 'lucide-react'
import type { FormEvent } from 'react'
import { useId, useState } from 'react'

export function RequestReadyDialog({
  balance,
  onConfirm,
  onOpenChange,
  open,
  pending,
  request,
}: {
  balance: number | null
  onConfirm: (stakeCredits: number | null) => Promise<boolean>
  onOpenChange: (open: boolean) => void
  open: boolean
  pending: boolean
  request: RequestSummary
}) {
  const inputId = useId()
  const publicAuthor = request.author_role === 'Public'
  const [stake, setStake] = useState(() => Math.min(25, Math.max(1, balance ?? 1)))
  const validStake = !publicAuthor || (
    Number.isInteger(stake) &&
    stake >= 1 &&
    stake <= 25 &&
    balance !== null &&
    stake <= balance
  )

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!validStake || pending) return
    if (await onConfirm(publicAuthor ? stake : null)) onOpenChange(false)
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
            <AlertDialogTitle>Ready for review</AlertDialogTitle>
            <AlertDialogDescription>
              Publish the current review package and place it in the maintainer queue.
            </AlertDialogDescription>
          </AlertDialogHeader>

          {request.first_ready_at_unix === null ? (
            <p className="border-y border-warning/30 bg-warning/5 px-3 py-2 text-sm leading-5">
              This is the first publication. The request remains public if it later returns to Working.
            </p>
          ) : null}

          {publicAuthor ? (
            <div className="grid gap-2">
              <label className="text-sm font-medium" htmlFor={inputId}>
                Review stake · 1–25 credits
              </label>
              <Input
                aria-describedby={`${inputId}-help`}
                aria-invalid={!validStake}
                id={inputId}
                max={Math.min(25, balance ?? 25)}
                min={1}
                onChange={(event) => setStake(Number(event.target.value))}
                type="number"
                value={stake}
              />
              <p className="text-xs leading-5 text-muted-foreground" id={`${inputId}-help`}>
                Private balance: {balance ?? 'unavailable'} credits. {stake} credits will be held now; Accepted work earns a reward after review.
              </p>
              {!validStake ? (
                <p className="text-sm text-destructive" role="alert">
                  Choose a whole number from 1 to 25 within your available balance.
                </p>
              ) : null}
            </div>
          ) : (
            <p className="text-sm leading-6 text-muted-foreground">
              Maintainer-authored requests do not use credits.
            </p>
          )}

          <AlertDialogFooter>
            <AlertDialogCancel disabled={pending} size="sm">
              Cancel
            </AlertDialogCancel>
            <Button disabled={!validStake || pending} size="sm" type="submit">
              {pending ? <LoaderCircle className="animate-spin" /> : null}
              Mark ready
            </Button>
          </AlertDialogFooter>
        </form>
      </AlertDialogContent>
    </AlertDialog>
  )
}
