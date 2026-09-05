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

type InitialRunView = {
  selectedJobKey: string | null
  selection: StepSelection | null
}

const MAX_CACHED_STEP_LOG_BYTES = 512 * 1_024
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
 * Cold-load selection so a failed run opens on its failure with zero clicks.
 * Failures without a failed step still open their job so setup errors and
 * terminal attempt state are visible without pretending another step failed.
 */
export function selectInitialView(
  jobs: readonly JobLike[],
): InitialRunView {
  const failedJob = jobs.find(({ job }) => job.state === 'failed')
  const failedAttempt = failedJob ? lastAttempt(failedJob) : null
  const failedStep = failedAttempt?.steps.find((step) => step.state === 'failed')
  if (failedJob && failedAttempt && failedStep) {
    return {
      selectedJobKey: failedJob.job.key,
      selection: {
        attemptId: failedAttempt.id,
        jobKey: failedJob.job.key,
        stepIndex: failedStep.index,
      },
    }
  }
  if (failedJob) {
    return { selectedJobKey: failedJob.job.key, selection: null }
  }

  for (const jobDetail of jobs) {
    const attempt = lastAttempt(jobDetail)
    const runningStep = attempt?.steps.find((step) => step.state === 'running')
    if (attempt && runningStep) {
      return {
        selectedJobKey: jobDetail.job.key,
        selection: {
          attemptId: attempt.id,
          jobKey: jobDetail.job.key,
          stepIndex: runningStep.index,
        },
      }
    }
  }

  const lastJob = jobs.at(-1)
  const lastJobAttempt = lastJob ? lastAttempt(lastJob) : null
  const lastStep = lastJobAttempt?.steps.at(-1)
  if (lastJob && lastJobAttempt && lastStep) {
    return {
      selectedJobKey: lastJob.job.key,
      selection: {
        attemptId: lastJobAttempt.id,
        jobKey: lastJob.job.key,
        stepIndex: lastStep.index,
      },
    }
  }

  return { selectedJobKey: null, selection: null }
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

type StepLogLike = { position: number; text: string; byte_length: number }

export function mergeStepLogPage<T extends StepLogLike>(
  previous: { logs: readonly T[]; hasEarlier: boolean },
  page: { logs: readonly T[]; has_earlier: boolean },
  after: number | undefined,
) {
  const merged = mergeStepLogs(after === undefined ? [] : previous.logs, page.logs)
  return {
    logs: merged.logs,
    // Forward pages can have earlier rows already present in the retained window.
    hasEarlier: merged.truncated || (after === undefined ? page.has_earlier : previous.hasEarlier),
  }
}

export function mergeStepLogs<T extends StepLogLike>(
  previous: readonly T[],
  incoming: readonly T[],
) {
  const lastPosition = previous.at(-1)?.position ?? 0
  // Pages and retained output are already ordered; reconnect overlap only needs
  // filtering against the last retained position, not a full map and sort.
  const ordered = [...previous, ...incoming.filter((log) => log.position > lastPosition)]
  let retainedBytes = 0
  let firstRetained = ordered.length
  while (firstRetained > 0) {
    const nextSize = ordered[firstRetained - 1]?.byte_length ?? 0
    if (
      firstRetained < ordered.length &&
      retainedBytes + nextSize > MAX_CACHED_STEP_LOG_BYTES
    ) break
    retainedBytes += nextSize
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

/**
 * What the reader is currently looking at. `selectedJobKey` and `selection`
 * must always agree, because refresh reconciliation derives the open job from
 * the selection; letting them drift snaps the page back after every poll.
 */
export type RunSelectionState = {
  attemptOverrides: Readonly<Record<string, string>>
  manualSelection: boolean
  selectedJobKey: string | null
  selection: StepSelection | null
}

/** Opening a job, or closing the one already open. */
export function selectJob<State extends RunSelectionState>(
  current: State,
  jobKey: string,
): State {
  const selectedJobKey = current.selectedJobKey === jobKey ? null : jobKey
  return {
    ...current,
    manualSelection: true,
    selectedJobKey,
    selection: current.selection?.jobKey === selectedJobKey
      ? current.selection
      : null,
  }
}

/** Switching which attempt of a job is on screen. */
export function selectAttempt<State extends RunSelectionState>(
  current: State,
  jobKey: string,
  attemptId: string,
): State {
  return {
    ...current,
    attemptOverrides: { ...current.attemptOverrides, [jobKey]: attemptId },
    manualSelection: true,
    selection: current.selection?.jobKey === jobKey ? null : current.selection,
  }
}

/** Opening a step's output, or closing the one already open. */
export function selectStep<State extends RunSelectionState>(
  current: State,
  selection: StepSelection,
): State {
  const open = current.selection
  const alreadyOpen = open !== null &&
    open.jobKey === selection.jobKey &&
    open.attemptId === selection.attemptId &&
    open.stepIndex === selection.stepIndex
  return {
    ...current,
    manualSelection: true,
    selectedJobKey: alreadyOpen ? current.selectedJobKey : selection.jobKey,
    selection: alreadyOpen ? null : selection,
  }
}
