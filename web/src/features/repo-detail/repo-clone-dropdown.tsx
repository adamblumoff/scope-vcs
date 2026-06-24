import type {
  RepoCloneCredentialView,
  RepoSummary,
} from '@/api/types'
import { CopyableCodeBlock } from '@/components/copyable-code-block'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { ChevronDown, Code2, LoaderCircle } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import {
  credentialedCloneCommand,
  publicCloneCommand,
} from './clone-command'

export function RepoCloneDropdown({
  cloneRemoteUrl,
  loadCloneCredential,
  repo,
}: {
  cloneRemoteUrl: string
  loadCloneCredential: () => Promise<RepoCloneCredentialView>
  repo: RepoSummary
}) {
  const [open, setOpen] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [credential, setCredential] =
    useState<RepoCloneCredentialView | null>(null)
  const rootRef = useRef<HTMLDivElement>(null)
  const permissioned = repo.role !== null
  const credentialSecret = credential?.token.secret ?? null
  const credentialedCommandReady =
    permissioned && credential !== null && credentialSecret !== null
  const cloneCommand = credentialedCommandReady
    ? credentialedCloneCommand(credential, credentialSecret)
    : publicCloneCommand({ git_remote_url: cloneRemoteUrl })

  useEffect(() => {
    if (!open) {
      return
    }

    function closeOnOutsidePointer(event: MouseEvent) {
      if (
        rootRef.current &&
        !rootRef.current.contains(event.target as Node)
      ) {
        setOpen(false)
      }
    }

    document.addEventListener('mousedown', closeOnOutsidePointer)
    return () =>
      document.removeEventListener('mousedown', closeOnOutsidePointer)
  }, [open])

  async function ensureCloneCredential() {
    if (!permissioned || credential || busy) {
      return
    }

    setBusy(true)
    setError(null)
    try {
      const nextCredential = await loadCloneCredential()
      if (!nextCredential.token.secret) {
        throw new Error('Clone credential did not include a secret')
      }
      setCredential(nextCredential)
    } catch (cloneError) {
      setError(
        cloneError instanceof Error
          ? cloneError.message
          : 'Clone credential failed',
      )
    } finally {
      setBusy(false)
    }
  }

  function toggleOpen() {
    const nextOpen = !open
    setOpen(nextOpen)
    if (nextOpen) {
      void ensureCloneCredential()
    }
  }

  return (
    <div className="relative" ref={rootRef}>
      <Button
        aria-expanded={open}
        aria-haspopup="dialog"
        onClick={toggleOpen}
        size="sm"
        type="button"
        variant="secondary"
      >
        <Code2 className="size-3.5" />
        <span>Clone</span>
        <ChevronDown
          className={cn(
            'size-3.5 text-muted-foreground transition-transform',
            open && 'rotate-180',
          )}
        />
      </Button>

      {open && (
        <div
          className="absolute right-0 top-full z-50 mt-2 w-[min(420px,calc(100vw-2rem))] rounded-md border border-border bg-popover p-3 text-popover-foreground shadow-md"
          role="dialog"
        >
          <div className="mb-2 flex h-6 items-center justify-between text-xs font-semibold leading-4">
            <span>HTTPS</span>
            {permissioned && busy && (
              <LoaderCircle className="size-3.5 animate-spin text-muted-foreground" />
            )}
          </div>
          {permissioned && !credentialedCommandReady ? (
            <div className="flex min-h-16 items-center gap-2 rounded-md border border-border bg-muted px-3 py-2 text-sm leading-5 text-muted-foreground">
              {busy && <LoaderCircle className="size-3.5 animate-spin" />}
              <span>
                {busy
                  ? 'Preparing clone command'
                  : 'Clone command unavailable'}
              </span>
            </div>
          ) : (
            <CopyableCodeBlock
              copyLabel="Copy clone command"
              value={cloneCommand}
            />
          )}
          {error && (
            <p className="mt-2 text-sm leading-5 text-destructive">
              {error}
            </p>
          )}
        </div>
      )}
    </div>
  )
}
