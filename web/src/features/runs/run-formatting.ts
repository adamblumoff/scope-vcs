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
