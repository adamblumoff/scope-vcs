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
import { LoaderCircle, Trash2 } from 'lucide-react'

export function DestructiveActionDialog({
  confirmLabel,
  description,
  onConfirm,
  onOpenChange,
  open,
  pending,
  subject,
  title,
}: {
  confirmLabel: string
  description: string
  onConfirm: () => void
  onOpenChange: (open: boolean) => void
  open: boolean
  pending: boolean
  subject: string
  title: string
}) {
  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{title}</AlertDialogTitle>
          <div className="break-all font-mono text-xs leading-5 text-muted-foreground">
            {subject}
          </div>
        </AlertDialogHeader>
        <AlertDialogDescription>{description}</AlertDialogDescription>
        <AlertDialogFooter>
          <AlertDialogCancel disabled={pending} size="sm">
            Cancel
          </AlertDialogCancel>
          <Button
            disabled={pending}
            onClick={onConfirm}
            size="sm"
            type="button"
            variant="destructive"
          >
            {pending ? (
              <LoaderCircle className="size-3.5 animate-spin" />
            ) : (
              <Trash2 className="size-3.5" />
            )}
            <span>{confirmLabel}</span>
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
