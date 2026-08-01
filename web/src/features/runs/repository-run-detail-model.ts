import type { RepoRunState } from '@/api/types'

type StepLike = {
  index: number
  state: string
}

type SelectionLike = {
  attemptId: string
  stepIndex: number
}

type AttemptLike = {
  id: string
  steps: readonly StepLike[]
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
    : { attemptId: attempt.id, stepIndex }
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
