import type { RepoRun } from '@/api/types'

type RunLookup = {
  runs: Array<Pick<RepoRun, 'id' | 'state'>>
}

const ACTIVE_RUN_STATES = new Set<RepoRun['state']>([
  'leased',
  'queued',
  'running',
])

export function shouldRefreshSelectedRunDetail(
  operations: RunLookup | null,
  selectedRunId: string | null,
) {
  return selectedRunId !== null &&
    operations?.runs.some(
      (run) => run.id === selectedRunId && ACTIVE_RUN_STATES.has(run.state),
    ) === true
}

export function selectedRunIsUnavailable(
  operations: RunLookup | null,
  selectedRunId: string | null,
) {
  return selectedRunId !== null &&
    (operations === null ||
      !operations.runs.some((run) => run.id === selectedRunId))
}

export function selectedRunBecameTerminal(
  previous: RunLookup | null,
  next: RunLookup | null,
  selectedRunId: string | null,
) {
  return shouldRefreshSelectedRunDetail(previous, selectedRunId) &&
    !selectedRunIsUnavailable(next, selectedRunId) &&
    !shouldRefreshSelectedRunDetail(next, selectedRunId)
}
