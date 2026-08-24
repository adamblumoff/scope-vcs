import { cn } from '@/lib/utils'
import { Check, Circle, LoaderCircle, Minus, X } from 'lucide-react'
import { type RunTone, runStatus } from './run-status'
import type { RepoRunTerminalReason } from '@/api/types'

const TONE_TEXT_CLASS: Record<RunTone, string> = {
  danger: 'text-danger-strong',
  inert: 'text-muted-foreground',
  running: 'text-info-strong',
  success: 'text-success-strong',
  waiting: 'text-warning-strong',
}

const TONE_ICON: Record<RunTone, typeof Check> = {
  danger: X,
  inert: Minus,
  running: LoaderCircle,
  success: Check,
  waiting: Circle,
}

export function RunStatusIcon({
  className,
  state,
  terminalReason,
}: {
  className?: string
  state: string
  terminalReason?: RepoRunTerminalReason | null
}) {
  const status = runStatus(state, terminalReason)
  const Icon = TONE_ICON[status.tone]
  return (
    <Icon
      aria-label={status.label}
      className={cn(
        'size-3.5 shrink-0',
        TONE_TEXT_CLASS[status.tone],
        status.animated && 'animate-spin',
        className,
      )}
    />
  )
}
