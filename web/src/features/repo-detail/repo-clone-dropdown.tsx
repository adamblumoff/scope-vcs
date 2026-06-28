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
  const permissioned = repo.role !== null
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
          className="absolute right-0 top-full z-50 mt-2 w-[min(420px,calc(100vw-2rem))] rounded-xl border border-border bg-popover p-3 text-popover-foreground shadow-[var(--shadow-pop)]"
          role="dialog"
        >
          <div className="mb-2 flex h-6 items-center justify-between text-xs font-semibold leading-4">
            <span>{cloneLabel}</span>
          </div>
          <CopyableCodeBlock copyLabel={copyLabel} value={cloneCommand} />
        </div>
      )}
    </div>
  )
}
