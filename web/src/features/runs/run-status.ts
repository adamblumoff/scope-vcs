import type { RepoRunTerminalReason } from '@/api/types'

/**
 * The single status vocabulary for runs, jobs, attempts and steps. Every runs
 * surface reads labels and tones from here so a running run can never be
 * rendered the same way as one that already passed.
 */
export type RunTone = 'running' | 'success' | 'danger' | 'waiting' | 'inert'

type RunStatus = {
  animated: boolean
  label: string
  tone: RunTone
}

const TONES: Record<string, RunTone> = {
  blocked: 'waiting',
  canceled: 'inert',
  canceling: 'waiting',
  dispatching: 'waiting',
  failed: 'danger',
  lost: 'danger',
  pending: 'waiting',
  queued: 'waiting',
  running: 'running',
  skipped: 'inert',
  succeeded: 'success',
}

const LABELS: Record<string, string> = {
  dispatching: 'starting',
}

const TERMINAL_LABELS: Record<RepoRunTerminalReason['kind'], string | null> = {
  'canceled': null,
  'execution-lost': 'execution lost',
  'runtime-setup-failed': 'setup failed',
  'step-failed': null,
  'timed-out': 'timed out',
}

export function runStatus(
  state: string,
  terminalReason?: RepoRunTerminalReason | null,
): RunStatus {
  const tone = TONES[state] ?? 'waiting'
  const reasonLabel = terminalReason
    ? TERMINAL_LABELS[terminalReason.kind]
    : null
  return {
    animated: tone === 'running',
    label: reasonLabel ?? LABELS[state] ?? state,
    tone,
  }
}
