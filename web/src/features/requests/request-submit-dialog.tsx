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

export function RequestSubmitDialog({
  onConfirm,
  onOpenChange,
  open,
  pending,
}: {
  onConfirm: () => Promise<boolean>
  onOpenChange: (open: boolean) => void
  open: boolean
  pending: boolean
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
            <AlertDialogTitle>Submit request</AlertDialogTitle>
            <AlertDialogDescription>
              Send the current request to its maintainers. You can keep editing and pushing after submission.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={pending} size="sm">Cancel</AlertDialogCancel>
            <Button disabled={pending} size="sm" type="submit">
              {pending ? <LoaderCircle className="animate-spin" /> : null}
              Submit
            </Button>
          </AlertDialogFooter>
        </form>
      </AlertDialogContent>
    </AlertDialog>
  )
}
