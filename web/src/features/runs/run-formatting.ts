import type { RepoRunTrigger } from '@/api/types'
import type { RepositoryRunState } from '@/api/types.generated'
import { runCanChange } from './repository-run-detail-model'

export function createRunTimeFormatter(timeZone?: string) {
  return new Intl.DateTimeFormat('en-US', {
    dateStyle: 'medium',
    timeStyle: 'short',
    timeZone,
  })
}

export function runUnixTimeDate(value: number) {
  return new Date(value * 1_000)
}

export function runDisplayState(run: {
  cancellation_requested: boolean
  state: RepositoryRunState
}): RepositoryRunState | 'canceling' {
  return run.cancellation_requested && runCanChange(run.state)
    ? 'canceling'
    : run.state
}

/** Elapsed seconds rendered for scanning: `44s`, `3m 04s`, `1h 12m`. */
export function formatDuration(seconds: number) {
  const safe = Math.max(0, Math.round(seconds))
  if (safe < 60) return `${safe}s`
  const minutes = Math.floor(safe / 60)
  if (minutes < 60) {
    const remaining = safe % 60
    return remaining === 0
      ? `${minutes}m`
      : `${minutes}m ${String(remaining).padStart(2, '0')}s`
  }
  const hours = Math.floor(minutes / 60)
  const remaining = minutes % 60
  return remaining === 0
    ? `${hours}h`
    : `${hours}h ${String(remaining).padStart(2, '0')}m`
}

/**
 * How long a span took, or how long it has been running. Returns null when the
 * span has not started, so callers render a placeholder instead of a fake zero.
 */
export function elapsedDuration(
  start: number | null,
  end: number | null,
  nowUnix: number,
) {
  if (start === null) return null
  return formatDuration((end ?? nowUnix) - start)
}

const MINUTE = 60
const HOUR = 60 * MINUTE
const DAY = 24 * HOUR

/** Relative time for list scanning. The absolute time goes in a title. */
export function formatRelativeTime(value: number, nowUnix: number) {
  const seconds = nowUnix - value
  if (seconds < 0) return 'just now'
  if (seconds < MINUTE) return 'just now'
  if (seconds < HOUR) return `${Math.floor(seconds / MINUTE)}m ago`
  if (seconds < DAY) return `${Math.floor(seconds / HOUR)}h ago`
  if (seconds < 2 * DAY) return 'yesterday'
  if (seconds < 30 * DAY) return `${Math.floor(seconds / DAY)}d ago`
  return createRunTimeFormatter('UTC')
    .format(runUnixTimeDate(value))
    .replace(/,\s\d{1,2}:\d{2}\s(AM|PM)$/, '')
}

/** How a run started, in the words a reader would use. */
export function runTriggerLabel(trigger: RepoRunTrigger) {
  return trigger === 'push-main' ? 'push' : 'manual'
}
