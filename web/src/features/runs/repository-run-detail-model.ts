import type { RepoRunState } from '@/api/types'

type StepLike = {
  index: number
  state: string
}

type AttemptLike = {
  id: string
  number: number
  steps: readonly StepLike[]
}

type JobLike = {
  job: {
    key: string
    needs: readonly string[]
    state: string
  }
  attempts: readonly AttemptLike[]
}

export type StepSelection = {
  attemptId: string
  jobKey: string
  stepIndex: number
}

const MAX_CACHED_STEP_LOG_CHARACTERS = 512 * 1_024
const GRAPH_DEFAULT_JOB_COUNT = 3

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

/**
 * Cold-load selection so a failed run opens on its failure with zero clicks:
 * the first failed step of the first failed job, else the step currently
 * running, else the last step of the last job, else nothing.
 */
export function selectInitialStep(
  jobs: readonly JobLike[],
): StepSelection | null {
  const failedJob = jobs.find(({ job }) => job.state === 'failed')
  const failedAttempt = failedJob ? lastAttempt(failedJob) : null
  const failedStep = failedAttempt?.steps.find((step) => step.state === 'failed')
  if (failedJob && failedAttempt && failedStep) {
    return {
      attemptId: failedAttempt.id,
      jobKey: failedJob.job.key,
      stepIndex: failedStep.index,
    }
  }

  for (const jobDetail of jobs) {
    const attempt = lastAttempt(jobDetail)
    const runningStep = attempt?.steps.find((step) => step.state === 'running')
    if (attempt && runningStep) {
      return {
        attemptId: attempt.id,
        jobKey: jobDetail.job.key,
        stepIndex: runningStep.index,
      }
    }
  }

  const lastJob = jobs.at(-1)
  const lastJobAttempt = lastJob ? lastAttempt(lastJob) : null
  const lastStep = lastJobAttempt?.steps.at(-1)
  if (lastJob && lastJobAttempt && lastStep) {
    return {
      attemptId: lastJobAttempt.id,
      jobKey: lastJob.job.key,
      stepIndex: lastStep.index,
    }
  }

  return null
}

/**
 * Which attempt a job's steps come from: the selected step's attempt, else an
 * explicit switcher choice, else the most recent attempt.
 */
export function attemptForJob<Attempt extends AttemptLike>(
  jobDetail: { attempts: readonly Attempt[]; job: { key: string } },
  attemptOverrides: Readonly<Record<string, string>>,
  selection: StepSelection | null,
): Attempt | null {
  if (selection && selection.jobKey === jobDetail.job.key) {
    const selected = jobDetail.attempts.find((attempt) => attempt.id === selection.attemptId)
    if (selected) return selected
  }
  const overrideId = attemptOverrides[jobDetail.job.key]
  if (overrideId) {
    const overridden = jobDetail.attempts.find((attempt) => attempt.id === overrideId)
    if (overridden) return overridden
  }
  return latestAttempt(jobDetail.attempts)
}

export function reconcileAttemptOverrides(
  overrides: Readonly<Record<string, string>>,
  jobs: readonly JobLike[],
): Record<string, string> {
  const next: Record<string, string> = {}
  for (const { attempts, job } of jobs) {
    const attemptId = overrides[job.key]
    if (attemptId && attempts.some((attempt) => attempt.id === attemptId)) {
      next[job.key] = attemptId
    }
  }
  return next
}

/**
 * The graph view only earns its keep once dependencies make the job strip
 * hard to scan; otherwise the flat strip is faster to read.
 */
export function defaultShowGraph(jobs: readonly JobLike[]) {
  return jobs.length > GRAPH_DEFAULT_JOB_COUNT &&
    jobs.some(({ job }) => job.needs.length > 0)
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

/**
 * The run detail response returns attempts newest first, so "latest" is the
 * highest attempt number rather than a position in the array.
 */
function lastAttempt(jobDetail: JobLike) {
  return latestAttempt(jobDetail.attempts)
}

export function latestAttempt<Attempt extends { number: number }>(
  attempts: readonly Attempt[],
): Attempt | null {
  return attempts.reduce<Attempt | null>(
    (latest, attempt) =>
      latest === null || attempt.number > latest.number ? attempt : latest,
    null,
  )
}
