import type { RepoRunState } from '@/api/types'

type AttemptLike = {
  id: string
}

type JobLike = {
  job: {
    key: string
    state: string
  }
  attempts: readonly AttemptLike[]
}

const MAX_CACHED_STEP_LOG_CHARACTERS = 512 * 1_024

export function runCanChange(state: RepoRunState): boolean {
  switch (state) {
    case 'queued':
    case 'dispatching':
    case 'running':
      return true
    case 'succeeded':
    case 'failed':
    case 'canceled':
    case 'lost':
      return false
  }
}

export function reconcileExpandedAttempts(
  expanded: ReadonlySet<string>,
  nextAttemptIds: readonly string[],
) {
  return new Set(
    [...expanded].filter((attemptId) => nextAttemptIds.includes(attemptId)),
  )
}

export function reconcileExpandedJobs(
  expanded: ReadonlySet<string>,
  jobs: readonly JobLike[],
) {
  const jobKeys = jobs.map(({ job }) => job.key)
  return new Set([...expanded].filter((key) => jobKeys.includes(key)))
}

export function runAttempts(jobs: readonly JobLike[]) {
  return jobs.flatMap(({ attempts }) => attempts)
}

export function mergeStepLogs<T extends { position: number; text: string }>(
  previous: readonly T[],
  incoming: readonly T[],
) {
  const byPosition = new Map(previous.map((log) => [log.position, log]))
  for (const log of incoming) byPosition.set(log.position, log)
  const ordered = [...byPosition.values()].sort((left, right) =>
    left.position - right.position
  )
  let retainedCharacters = 0
  let firstRetained = ordered.length
  while (firstRetained > 0) {
    const nextSize = ordered[firstRetained - 1]?.text.length ?? 0
    if (
      firstRetained < ordered.length &&
      retainedCharacters + nextSize > MAX_CACHED_STEP_LOG_CHARACTERS
    ) break
    retainedCharacters += nextSize
    firstRetained -= 1
  }
  return {
    logs: ordered.slice(firstRetained),
    truncated: firstRetained > 0,
  }
}
