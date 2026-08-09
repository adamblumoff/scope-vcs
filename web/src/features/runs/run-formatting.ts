import type {
  RepositoryRunState,
  RunRunnerSelection,
} from '@/api/types.generated'

const DATE_FORMATTER = new Intl.DateTimeFormat('en-US', {
  dateStyle: 'medium',
  timeStyle: 'short',
  timeZone: 'UTC',
})

export function formatRunUnixTime(value: number) {
  return DATE_FORMATTER.format(new Date(value * 1_000))
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
  return run.cancellation_requested ? 'canceling' : run.state
}
