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
import { LoaderCircle } from 'lucide-react'
import type { FormEvent } from 'react'

export function RequestReadyDialog({
  onConfirm,
  onOpenChange,
  open,
  pending,
  request,
}: {
  onConfirm: () => Promise<boolean>
  onOpenChange: (open: boolean) => void
  open: boolean
  pending: boolean
  request: RequestSummary
}) {
  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!pending && await onConfirm()) onOpenChange(false)
  }

  return (
    <AlertDialog onOpenChange={(nextOpen) => !pending && onOpenChange(nextOpen)} open={open}>
      <AlertDialogContent asChild>
        <form onSubmit={(event) => void submit(event)}>
          <AlertDialogHeader>
            <AlertDialogTitle>Ready for review</AlertDialogTitle>
            <AlertDialogDescription>
              Publish the current request and place it in the maintainer queue.
            </AlertDialogDescription>
          </AlertDialogHeader>
          {request.first_ready_at_unix === null ? (
            <p className="border-y border-warning/30 bg-warning/5 px-3 py-2 text-sm leading-5">
              This is the first publication. The request remains visible if it later returns to Working.
            </p>
          ) : null}
          <AlertDialogFooter>
            <AlertDialogCancel disabled={pending} size="sm">Cancel</AlertDialogCancel>
            <Button disabled={pending} size="sm" type="submit">
              {pending ? <LoaderCircle className="animate-spin" /> : null}
              Mark ready
            </Button>
          </AlertDialogFooter>
        </form>
      </AlertDialogContent>
    </AlertDialog>
  )
}
