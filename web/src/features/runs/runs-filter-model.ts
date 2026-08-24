import type { RepoRunHistoryPage } from '@/api/types'
import { runDisplayState } from './run-formatting'
import { type RunTone, runStatus } from './run-status'

export type RunStatusFilter = 'any' | 'failed' | 'running' | 'succeeded'

const STATUS_FILTER_TONE: Partial<Record<RunStatusFilter, RunTone>> = {
  failed: 'danger',
  running: 'running',
  succeeded: 'success',
}

export const RUN_STATUS_FILTER_OPTIONS: {
  label: string
  value: RunStatusFilter
}[] = [
  { label: 'Any status', value: 'any' },
  { label: 'Running', value: 'running' },
  { label: 'Failed', value: 'failed' },
  { label: 'Succeeded', value: 'succeeded' },
]

export function runMatchesStatusFilter(
  run: RepoRunHistoryPage['runs'][number],
  filter: RunStatusFilter,
) {
  if (filter === 'any') return true
  return runStatus(runDisplayState(run)).tone === STATUS_FILTER_TONE[filter]
}
