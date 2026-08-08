import type { RepoRunState } from '@/api/types'

type StepLike = {
  index: number
  state: string
}

type SelectionLike = {
  jobKey: string
  attemptId: string
  stepIndex: number
}

type AttemptLike = {
  id: string
  steps: readonly StepLike[]
}

type JobLike = {
  job: {
    key: string
    state: string
  }
  attempts: readonly AttemptLike[]
}

const MAX_CACHED_STEP_LOG_CHARACTERS = 512 * 1_024

export function runNeedsPolling(state: RepoRunState): boolean {
  switch (state) {
    case 'queued':
    case 'leased':
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
  knownAttemptIds: readonly string[],
  nextAttemptIds: readonly string[],
) {
  const next = new Set(
    [...expanded].filter((attemptId) => nextAttemptIds.includes(attemptId)),
  )
  if (knownAttemptIds.length === 0) {
    if (nextAttemptIds[0]) next.add(nextAttemptIds[0])
    return next
  }
  const known = new Set(knownAttemptIds)
  for (const attemptId of nextAttemptIds) {
    if (!known.has(attemptId)) next.add(attemptId)
  }
  return next
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

export function newlySelectableAttempt(
  previousAttempts: readonly AttemptLike[],
  nextAttempts: readonly AttemptLike[],
) {
  const previousById = new Map(
    previousAttempts.map((attempt) => [attempt.id, attempt]),
  )
  return nextAttempts.find((attempt) => {
    const previous = previousById.get(attempt.id)
    return defaultSelectedStep(attempt.steps) !== null &&
      (previous === undefined || previous.steps.length === 0)
  })
}

export function defaultSelectedJob(jobs: readonly JobLike[]) {
  const preferredStates = [
    'running',
    'failed',
    'lost',
    'canceled',
    'leased',
    'queued',
    'blocked',
  ]
  for (const state of preferredStates) {
    const match = jobs.find(({ job }) => job.state === state)
    if (match) return match
  }
  return jobs[0] ?? null
}

export function defaultSelectedStep(steps: readonly StepLike[]) {
  const preferred = steps.find((step) => step.state === 'running')?.index
    ?? steps.find((step) => step.state === 'failed')?.index
    ?? steps.find((step) => step.state === 'canceled')?.index
    ?? steps.find((step) => step.state === 'lost')?.index
  if (preferred !== undefined) return preferred
  for (let index = steps.length - 1; index >= 0; index -= 1) {
    const step = steps[index]
    if (step?.state === 'succeeded') return step.index
  }
  return steps[0]?.index ?? null
}

export function reconcileAutomaticStepSelection(
  selection: SelectionLike | null,
  attempts: readonly AttemptLike[],
) {
  if (!selection) return null
  const attempt = attempts.find((candidate) => candidate.id === selection.attemptId)
  if (!attempt) return null
  const stepIndex = defaultSelectedStep(attempt.steps)
  if (stepIndex === null) return null
  return stepIndex === selection.stepIndex
    ? selection
    : { ...selection, attemptId: attempt.id, stepIndex }
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
