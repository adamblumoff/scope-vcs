import type { RepoSummary } from '@/api/types'
import { CopyableCodeBlock } from '@/components/copyable-code-block'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { ChevronDown, Code2 } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import {
  permissionedCloneCommand,
  publicCloneCommand,
} from './clone-command'

export function RepoCloneDropdown({
  cloneRemoteUrl,
  repo,
}: {
  cloneRemoteUrl: string
  repo: RepoSummary
}) {
  const [open, setOpen] = useState(false)
  const rootRef = useRef<HTMLDivElement>(null)
  const permissioned = repo.access.actor !== 'Public'
  const cloneCommand = permissioned
    ? permissionedCloneCommand(repo.owner_handle, repo.name)
    : publicCloneCommand(cloneRemoteUrl)
  const cloneLabel = permissioned ? 'Scope CLI' : 'Public HTTPS'
  const copyLabel = permissioned
    ? 'Copy permissioned clone command'
    : 'Copy public clone command'

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

  function toggleOpen() {
    setOpen(!open)
  }

  return (
    <div className="relative" ref={rootRef}>
      <Button
        aria-expanded={open}
        aria-haspopup="dialog"
        onClick={toggleOpen}
        size="sm"
        type="button"
        variant="default"
      >
        <Code2 className="size-3.5" />
        <span>Clone</span>
        <span className="-my-2 ml-1 flex h-8 items-center border-l border-black/15 pl-2">
          <ChevronDown
            className={cn(
            'size-3.5 text-primary-foreground/70 transition-transform',
            open && 'rotate-180',
          )}
          />
        </span>
      </Button>

      {open && (
        <dialog
          aria-label={`${cloneLabel} clone command`}
          className="absolute left-auto right-0 top-full z-50 m-0 mt-2 w-[min(420px,calc(100vw-2rem))] rounded-lg border border-[var(--border-strong)] bg-popover p-3 text-popover-foreground shadow-[var(--shadow-pop)]"
          open
        >
          <div className="mb-2 flex h-6 items-center justify-between text-xs font-semibold leading-4">
            <span>{cloneLabel}</span>
          </div>
          <CopyableCodeBlock copyLabel={copyLabel} value={cloneCommand} />
        </dialog>
      )}
    </div>
  )
}
