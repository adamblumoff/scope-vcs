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
import type { RequestSummary } from '@/api/types'
import { LoaderCircle } from 'lucide-react'
import type { FormEvent } from 'react'

export function RequestSubmitDialog({
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
  const publicRequest = request.author_role === 'Public'
  const title = publicRequest ? 'Request maintainer review?' : 'Mark request ready?'
  const description = publicRequest
    ? 'Send the current request to the repository maintainers for review. You can keep editing and pushing afterward.'
    : 'Mark the current maintainer request ready to merge. You can keep editing and pushing afterward.'
  const confirmLabel = publicRequest ? 'Request review' : 'Mark ready'
  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!pending && await onConfirm()) onOpenChange(false)
  }

  return (
    <AlertDialog onOpenChange={(nextOpen) => !pending && onOpenChange(nextOpen)} open={open}>
      <AlertDialogContent asChild>
        <form onSubmit={(event) => void submit(event)}>
          <AlertDialogHeader>
            <AlertDialogTitle>{title}</AlertDialogTitle>
            <AlertDialogDescription>
              {description}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={pending} size="sm">Cancel</AlertDialogCancel>
            <Button disabled={pending} size="sm" type="submit">
              {pending ? <LoaderCircle className="animate-spin" /> : null}
              {confirmLabel}
            </Button>
          </AlertDialogFooter>
        </form>
      </AlertDialogContent>
    </AlertDialog>
  )
}
