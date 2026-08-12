import type {
  RepositoryRunState,
  RunRunnerSelection,
} from '@/api/types.generated'
import { runNeedsPolling } from './repository-run-detail-model'

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

export function formatRunRunnerSelection(selection: RunRunnerSelection) {
  switch (selection.kind) {
    case 'any':
      return 'any runner'
    case 'named':
      return selection.name
    case 'mixed':
      return 'multiple runners'
  }
}

export function runDisplayState(run: {
  cancellation_requested: boolean
  state: RepositoryRunState
}): RepositoryRunState | 'canceling' {
  return run.cancellation_requested && runNeedsPolling(run.state)
    ? 'canceling'
    : run.state
}
