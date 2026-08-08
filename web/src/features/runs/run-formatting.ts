import type { RunRunnerSelection } from '@/api/types.generated'

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
