import type { RepoSummary } from '@/api/types'
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
import { AlertTriangle, LoaderCircle, Trash2 } from 'lucide-react'
import type { FormEvent } from 'react'
import { useId, useState } from 'react'

export function DeleteRepositoryDialog({
  onCancel,
  onConfirm,
  repo,
}: {
  onCancel: () => void
  onConfirm: (repo: RepoSummary) => Promise<void>
  repo: RepoSummary
}) {
  const inputId = useId()
  const [confirmed, setConfirmed] = useState(false)
  const [typedName, setTypedName] = useState('')
  const [busy, setBusy] = useState(false)
  const canDelete = typedName === repo.name

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!confirmed) {
      setConfirmed(true)
      return
    }

    if (!confirmed || !canDelete || busy) {
      return
    }

    setBusy(true)
    try {
      await onConfirm(repo)
    } catch {
      setBusy(false)
    }
  }

  return (
    <AlertDialog
      open
      onOpenChange={(open) => {
        if (!open && !busy) {
          onCancel()
        }
      }}
    >
      <AlertDialogContent asChild>
        <form onSubmit={(event) => void submit(event)}>
          <AlertDialogHeader className="grid-cols-[auto_minmax(0,1fr)] gap-x-3">
            <div className="row-span-2 flex size-9 shrink-0 items-center justify-center rounded-xl bg-destructive/10 text-destructive">
              <AlertTriangle className="size-4" />
            </div>
            <AlertDialogTitle>Delete repository</AlertDialogTitle>
            <div className="break-all font-mono text-xs leading-5 text-muted-foreground">
              {repo.id}
            </div>
          </AlertDialogHeader>

          {!confirmed ? (
            <AlertDialogDescription>
              This permanently removes the repo and stored Git data from Scope.
            </AlertDialogDescription>
          ) : (
            <div className="space-y-2">
              <AlertDialogDescription>
                Type{' '}
                <span className="font-mono text-foreground">{repo.name}</span>{' '}
                to permanently delete this repository.
              </AlertDialogDescription>
              <Input
                aria-label={`Type ${repo.name} to permanently delete this repository`}
                autoFocus
                className="font-mono"
                id={inputId}
                onChange={(event) => setTypedName(event.target.value)}
                value={typedName}
              />
            </div>
          )}

          <AlertDialogFooter>
            {!confirmed ? (
              <>
                <AlertDialogCancel size="sm" variant="secondary">
                  Cancel
                </AlertDialogCancel>
                <Button size="sm" type="submit">
                  Continue
                </Button>
              </>
            ) : (
              <>
                <Button
                  disabled={busy}
                  onClick={() => setConfirmed(false)}
                  size="sm"
                  type="button"
                  variant="secondary"
                >
                  Back
                </Button>
                <Button
                  disabled={!canDelete || busy}
                  size="sm"
                  type="submit"
                  variant="destructive"
                >
                  {busy ? (
                    <LoaderCircle className="size-3.5 animate-spin" />
                  ) : (
                    <Trash2 className="size-3.5" />
                  )}
                  <span>Delete</span>
                </Button>
              </>
            )}
          </AlertDialogFooter>
        </form>
      </AlertDialogContent>
    </AlertDialog>
  )
}
