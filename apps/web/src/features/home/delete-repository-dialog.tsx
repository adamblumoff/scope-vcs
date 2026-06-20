import type { RepoSummary } from '@/api/types'
import { Button } from '@/components/ui/button'
import { AlertTriangle, LoaderCircle, Trash2 } from 'lucide-react'
import type { FormEvent } from 'react'
import { useState } from 'react'

export function DeleteRepositoryDialog({
  onCancel,
  onConfirm,
  repo,
}: {
  onCancel: () => void
  onConfirm: (repo: RepoSummary) => Promise<void>
  repo: RepoSummary
}) {
  const [confirmed, setConfirmed] = useState(false)
  const [typedName, setTypedName] = useState('')
  const [busy, setBusy] = useState(false)
  const canDelete = typedName === repo.name

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
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
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-background/80 px-4 backdrop-blur-sm">
      <div className="w-full max-w-[520px] rounded-md border border-border bg-background p-5 shadow-lg">
        <div className="flex items-start gap-3">
          <div className="flex size-9 shrink-0 items-center justify-center rounded-md border border-destructive/50 text-destructive">
            <AlertTriangle className="size-4" />
          </div>
          <div className="min-w-0">
            <div className="text-sm font-semibold leading-5">
              Delete repository
            </div>
            <div className="mt-1 break-all font-mono text-xs leading-5 text-muted-foreground">
              {repo.id}
            </div>
          </div>
        </div>

        {!confirmed ? (
          <div className="mt-5 space-y-5">
            <p className="text-sm leading-5 text-muted-foreground">
              This permanently removes the repo, pending review state, and stored
              Git data from Scope.
            </p>
            <div className="flex justify-end gap-2">
              <Button onClick={onCancel} size="sm" type="button" variant="secondary">
                Cancel
              </Button>
              <Button onClick={() => setConfirmed(true)} size="sm" type="button">
                Continue
              </Button>
            </div>
          </div>
        ) : (
          <form className="mt-5 space-y-5" onSubmit={(event) => void submit(event)}>
            <label className="block text-sm leading-5 text-muted-foreground">
              Type <span className="font-mono text-foreground">{repo.name}</span> to
              permanently delete this repository.
            </label>
            <input
              autoFocus
              className="h-9 w-full rounded-md border border-input bg-background px-3 font-mono text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
              onChange={(event) => setTypedName(event.target.value)}
              value={typedName}
            />
            <div className="flex justify-end gap-2">
              <Button
                disabled={busy}
                onClick={() => setConfirmed(false)}
                size="sm"
                type="button"
                variant="secondary"
              >
                Back
              </Button>
              <Button disabled={!canDelete || busy} size="sm" type="submit">
                {busy ? (
                  <LoaderCircle className="size-3.5 animate-spin" />
                ) : (
                  <Trash2 className="size-3.5" />
                )}
                <span>Delete</span>
              </Button>
            </div>
          </form>
        )}
      </div>
    </div>
  )
}
