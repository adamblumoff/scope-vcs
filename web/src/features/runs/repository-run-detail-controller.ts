import type {
  RepoRunAttempt,
  RepoRunDetail,
  RepoRunJobDetail,
  RepoRunLog,
  RepoRunStepLogPage,
  RunActionInput,
  RunStepLogsInput,
} from '@/api/types'
import {
  useCallback,
  useEffect,
  useReducer,
  useRef,
} from 'react'
import {
  defaultSelectedStep,
  defaultSelectedJob,
  mergeStepLogs,
  reconcileAutomaticStepSelection,
  reconcileExpandedAttempts,
  reconcileExpandedJobs,
  runAttempts,
  runNeedsPolling,
} from './repository-run-detail-model'

const REFRESH_INTERVAL_MS = 2_000
const MAX_CACHED_LOG_STEPS = 8

export type StepSelection = {
  jobKey: string
  attemptId: string
  stepIndex: number
}

export type StepLogState = {
  error: string | null
  loading: boolean
  logs: RepoRunLog[]
  logsTruncated: boolean
  nextAfter: number
}

type DetailViewState = {
  actionError: string | null
  detail: RepoRunDetail
  expandedAttempts: Set<string>
  expandedJobs: Set<string>
  logStates: Record<string, StepLogState>
  metadataError: string | null
  pendingAction: 'cancel' | 'retry' | null
  pendingAutomaticSelection: StepSelection | null
  reconciliationGeneration: number | null
  selection: StepSelection | null
  selectionIsAutomatic: boolean
}

type DetailViewUpdate = (state: DetailViewState) => DetailViewState

const EMPTY_LOG_STATE: StepLogState = {
  error: null,
  loading: false,
  logs: [],
  logsTruncated: false,
  nextAfter: 0,
}

function createDetailViewState(detail: RepoRunDetail): DetailViewState {
  const attempts = runAttempts(detail.jobs)
  const attemptIds = attempts.map((attempt) => attempt.id)
  const initialJob = defaultSelectedJob(detail.jobs)
  const initialAttempt = initialJob?.attempts[0]
  const initialStepIndex = initialAttempt
    ? defaultSelectedStep(initialAttempt.steps)
    : null
  return {
    actionError: null,
    detail,
    expandedAttempts: initialAttempt
      ? new Set([initialAttempt.id])
      : reconcileExpandedAttempts(new Set(), [], attemptIds),
    expandedJobs: initialJob ? new Set([initialJob.job.key]) : new Set(),
    logStates: {},
    metadataError: null,
    pendingAction: null,
    pendingAutomaticSelection: null,
    reconciliationGeneration: null,
    selection: initialAttempt && initialStepIndex !== null
      ? {
          attemptId: initialAttempt.id,
          jobKey: initialJob.job.key,
          stepIndex: initialStepIndex,
        }
      : null,
    selectionIsAutomatic: true,
  }
}

function updateDetailView(
  state: DetailViewState,
  update: DetailViewUpdate,
) {
  return update(state)
}

export function useRepositoryRunDetailController({
  initialDetail,
  loadDetail,
  loadLogs,
  params,
}: {
  initialDetail: RepoRunDetail
  loadDetail: () => Promise<RepoRunDetail>
  loadLogs: (input: RunStepLogsInput) => Promise<RepoRunStepLogPage>
  params: RunActionInput
}) {
  const [view, updateView] = useReducer(
    updateDetailView,
    initialDetail,
    createDetailViewState,
  )
  const detailInFlightRef = useRef<Promise<void> | null>(null)
  const detailGenerationRef = useRef(0)
  const knownAttemptIdsRef = useRef<string[] | null>(null)
  if (knownAttemptIdsRef.current === null) {
    knownAttemptIdsRef.current = runAttempts(initialDetail.jobs).map(
      (attempt) => attempt.id,
    )
  }
  const logInFlightRef = useRef<Map<string, Promise<boolean>> | null>(null)
  if (logInFlightRef.current === null) {
    logInFlightRef.current = new Map()
  }
  const logStatesRef = useRef(view.logStates)
  const mountedRef = useRef(false)

  useEffect(() => {
    logStatesRef.current = view.logStates
  }, [view.logStates])

  const refreshDetail = useCallback(async (forceAfterInFlight = false) => {
    if (detailInFlightRef.current) {
      try {
        await detailInFlightRef.current
      } catch (error) {
        if (forceAfterInFlight) throw error
        return
      }
      if (!forceAfterInFlight) return
    }
    const generation = ++detailGenerationRef.current
    const request = loadDetail()
      .then((nextDetail) => {
        if (!mountedRef.current) return
        const nextAttempts = runAttempts(nextDetail.jobs)
        const nextIds = nextAttempts.map((attempt) => attempt.id)
        const previousIds = knownAttemptIdsRef.current ?? []
        const newAttempt = nextAttempts.find((attempt) =>
          !previousIds.includes(attempt.id)
        )
        knownAttemptIdsRef.current = nextIds
        updateView((current) => {
          let nextSelection = current.selection
          let pendingAutomaticSelection = current.pendingAutomaticSelection
          let selectionIsAutomatic = current.selectionIsAutomatic
          const currentAttempts = runAttempts(current.detail.jobs)
          const newest = newAttempt ?? nextAttempts[0]
          const previousNewest = currentAttempts.find((attempt) =>
            attempt.id === newest?.id
          )
          const newestGainedSteps = newest?.id === previousNewest?.id &&
            previousNewest.steps.length === 0 &&
            newest.steps.length > 0
          if (
            current.selectionIsAutomatic &&
            (newAttempt !== undefined || newestGainedSteps) &&
            newest
          ) {
            const stepIndex = defaultSelectedStep(newest.steps)
            if (stepIndex !== null) {
              const jobKey = attemptJobKey(nextDetail.jobs, newest.id)
              if (!jobKey) return current
              const candidate = { attemptId: newest.id, jobKey, stepIndex }
              if (sameSelection(current.selection, candidate)) {
                pendingAutomaticSelection = null
              } else if (
                current.selection &&
                current.selectionIsAutomatic
              ) {
                pendingAutomaticSelection = candidate
              } else {
                nextSelection = candidate
                pendingAutomaticSelection = null
              }
              selectionIsAutomatic = true
            }
          } else if (selectionIsAutomatic) {
            if (
              pendingAutomaticSelection &&
              selectionExists(
                pendingAutomaticSelection,
                nextDetail.jobs,
              )
            ) {
              // Keep draining the prior selection before the pending handoff.
            } else {
              const candidate = reconcileAutomaticStepSelection(
                current.selection,
                nextAttempts,
              )
              if (
                candidate &&
                current.selection &&
                !sameSelection(candidate, current.selection)
              ) {
                pendingAutomaticSelection = sameSelection(
                    pendingAutomaticSelection,
                    candidate,
                  )
                  ? pendingAutomaticSelection
                  : candidate
              } else {
                nextSelection = candidate
                pendingAutomaticSelection = null
              }
            }
          }
          if (
            !selectionIsAutomatic &&
            nextSelection &&
            !selectionExists(nextSelection, nextDetail.jobs)
          ) {
            nextSelection = null
            pendingAutomaticSelection = null
          }
          const reconciledAction = current.reconciliationGeneration !== null &&
            generation >= current.reconciliationGeneration
          const expandedJobs = reconcileExpandedJobs(
            current.expandedJobs,
            nextDetail.jobs,
          )
          if (newAttempt && current.selectionIsAutomatic) {
            const jobKey = attemptJobKey(nextDetail.jobs, newAttempt.id)
            if (jobKey) expandedJobs.add(jobKey)
          }
          return {
            ...current,
            detail: nextDetail,
            expandedAttempts: reconcileExpandedAttempts(
              current.expandedAttempts,
              previousIds,
              nextIds,
            ),
            expandedJobs,
            metadataError: null,
            pendingAction: reconciledAction ? null : current.pendingAction,
            pendingAutomaticSelection,
            reconciliationGeneration: reconciledAction
              ? null
              : current.reconciliationGeneration,
            selection: nextSelection,
            selectionIsAutomatic,
          }
        })
      })
      .catch((error: unknown) => {
        if (mountedRef.current) {
          updateView((current) => ({
            ...current,
            metadataError: errorMessage(error),
          }))
        }
        if (forceAfterInFlight) throw error
      })
      .finally(() => {
        if (detailInFlightRef.current === request) {
          detailInFlightRef.current = null
        }
      })
    detailInFlightRef.current = request
    return request
  }, [loadDetail])

  const refreshLogs = useCallback((target: StepSelection) => {
    const key = stepKey(target)
    const inFlight = logInFlightRef.current
    if (!inFlight) return
    const existing = inFlight.get(key)
    if (existing) return existing
    const current = logStatesRef.current[key] ?? EMPTY_LOG_STATE
    const loadingState = {
      ...current,
      error: null,
      loading: true,
    }
    logStatesRef.current = withBoundedLogStates(
      logStatesRef.current,
      key,
      loadingState,
    )
    updateView((state) => ({
      ...state,
      logStates: withBoundedLogStates(
        state.logStates,
        key,
        loadingState,
      ),
    }))
    const request = loadLogs({
      ...params,
      after: current.nextAfter,
      attempt_id: target.attemptId,
      step_index: target.stepIndex,
    })
      .then((page) => {
        if (!mountedRef.current) return false
        const previous = logStatesRef.current[key] ?? current
        const merged = mergeStepLogs(previous.logs, page.logs)
        const nextState = {
          error: null,
          loading: false,
          logs: merged.logs,
          logsTruncated: previous.logsTruncated ||
            page.logs_truncated ||
            merged.truncated,
          nextAfter: page.next_after,
        }
        logStatesRef.current = withBoundedLogStates(
          logStatesRef.current,
          key,
          nextState,
        )
        updateView((state) => ({
          ...state,
          logStates: withBoundedLogStates(
            state.logStates,
            key,
            nextState,
          ),
        }))
        return true
      })
      .catch((error: unknown) => {
        if (!mountedRef.current) return false
        const errorState = {
          ...(logStatesRef.current[key] ?? EMPTY_LOG_STATE),
          error: errorMessage(error),
          loading: false,
        }
        logStatesRef.current = withBoundedLogStates(
          logStatesRef.current,
          key,
          errorState,
        )
        updateView((state) => ({
          ...state,
          logStates: withBoundedLogStates(
            state.logStates,
            key,
            errorState,
          ),
        }))
        return false
      })
      .finally(() => {
        if (inFlight.get(key) === request) inFlight.delete(key)
      })
    inFlight.set(key, request)
    return request
  }, [loadLogs, params])

  const refreshLogsAfterInFlight = useCallback(async (
    target: StepSelection,
  ) => {
    const key = stepKey(target)
    const existing = logInFlightRef.current?.get(key)
    if (existing) await existing
    let previousAfter: number
    do {
      previousAfter = logStatesRef.current[key]?.nextAfter ?? 0
      if (!await refreshLogs(target)) return false
    } while (
      mountedRef.current &&
      (logStatesRef.current[key]?.nextAfter ?? 0) > previousAfter
    )
    return mountedRef.current
  }, [refreshLogs])

  useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
    }
  }, [])

  useEffect(() => {
    if (!runNeedsPolling(view.detail.run.state)) return
    const timer = window.setInterval(
      () => void refreshDetail(),
      REFRESH_INTERVAL_MS,
    )
    return () => window.clearInterval(timer)
  }, [refreshDetail, view.detail.run.state])

  useEffect(() => {
    const selection = view.selection
    if (!selection) return
    if (!runNeedsPolling(view.detail.run.state)) {
      void refreshLogsAfterInFlight(selection)
      return
    }
    void refreshLogs(selection)
    const timer = window.setInterval(
      () => void refreshLogs(selection),
      REFRESH_INTERVAL_MS,
    )
    return () => window.clearInterval(timer)
  }, [
    refreshLogs,
    refreshLogsAfterInFlight,
    view.detail.run.state,
    view.selection,
  ])

  useEffect(() => {
    const currentSelection = view.selection
    const pendingSelection = view.pendingAutomaticSelection
    if (!currentSelection || !pendingSelection) return
    let active = true
    const finishAutomaticSelection = async () => {
      if (!await refreshLogsAfterInFlight(currentSelection)) return
      if (active && mountedRef.current) {
        updateView((current) => {
          if (
            !sameSelection(current.selection, currentSelection) ||
            !sameSelection(current.pendingAutomaticSelection, pendingSelection)
          ) return current
          return {
            ...current,
            pendingAutomaticSelection: null,
            selection: pendingSelection,
          }
        })
      }
    }
    void finishAutomaticSelection()
    const timer = window.setInterval(
      () => void finishAutomaticSelection(),
      REFRESH_INTERVAL_MS,
    )
    return () => {
      active = false
      window.clearInterval(timer)
    }
  }, [
    refreshLogsAfterInFlight,
    view.pendingAutomaticSelection,
    view.selection,
  ])

  const performAction = useCallback(async (
    kind: 'cancel' | 'retry',
    action: () => Promise<void>,
  ) => {
    updateView((current) => ({
      ...current,
      actionError: null,
      pendingAction: kind,
    }))
    try {
      await action()
    } catch (error) {
      if (mountedRef.current) {
        updateView((current) => ({
          ...current,
          actionError: errorMessage(error),
          pendingAction: null,
        }))
      }
      return
    }
    const reconciliationGeneration = detailGenerationRef.current + 1
    updateView((current) => ({
      ...current,
      reconciliationGeneration,
    }))
    try {
      await refreshDetail(true)
    } catch {
      // The detail loader owns metadata errors. Keep controls disabled until a
      // post-mutation refresh reaches the required generation.
    }
  }, [refreshDetail])

  function toggleJob(jobDetail: RepoRunJobDetail) {
    updateView((current) => {
      const nextJobs = new Set(current.expandedJobs)
      const expanding = !nextJobs.has(jobDetail.job.key)
      if (expanding) nextJobs.add(jobDetail.job.key)
      else nextJobs.delete(jobDetail.job.key)
      const firstAttempt = jobDetail.attempts[0]
      const stepIndex = firstAttempt
        ? defaultSelectedStep(firstAttempt.steps)
        : null
      let nextSelection = current.selection
      let selectionIsAutomatic = current.selectionIsAutomatic
      if (expanding && firstAttempt && stepIndex !== null) {
        nextSelection = {
          attemptId: firstAttempt.id,
          jobKey: jobDetail.job.key,
          stepIndex,
        }
        selectionIsAutomatic = true
      } else if (!expanding && nextSelection?.jobKey === jobDetail.job.key) {
        nextSelection = null
        selectionIsAutomatic = false
      }
      return {
        ...current,
        expandedAttempts: expanding && firstAttempt
          ? new Set(current.expandedAttempts).add(firstAttempt.id)
          : current.expandedAttempts,
        expandedJobs: nextJobs,
        pendingAutomaticSelection: null,
        selection: nextSelection,
        selectionIsAutomatic,
      }
    })
  }

  function toggleAttempt(jobKey: string, attempt: RepoRunAttempt) {
    updateView((current) => {
      const next = new Set(current.expandedAttempts)
      const expanding = !next.has(attempt.id)
      if (next.has(attempt.id)) next.delete(attempt.id)
      else next.add(attempt.id)
      let nextSelection = current.selection
      let selectionIsAutomatic = current.selectionIsAutomatic
      if (expanding) {
        const stepIndex = defaultSelectedStep(attempt.steps)
        if (stepIndex !== null) {
          nextSelection = { attemptId: attempt.id, jobKey, stepIndex }
          selectionIsAutomatic = true
        }
      } else if (nextSelection?.attemptId === attempt.id) {
        nextSelection = null
        selectionIsAutomatic = false
      }
      return {
        ...current,
        expandedAttempts: next,
        pendingAutomaticSelection: null,
        selection: nextSelection,
        selectionIsAutomatic,
      }
    })
  }

  function toggleStep(jobKey: string, attemptId: string, stepIndex: number) {
    updateView((current) => ({
      ...current,
      pendingAutomaticSelection: null,
      selection: current.selection?.jobKey === jobKey &&
        current.selection.attemptId === attemptId &&
        current.selection.stepIndex === stepIndex
        ? null
        : { attemptId, jobKey, stepIndex },
      selectionIsAutomatic: false,
    }))
  }

  const selectedLogState = view.selection
    ? view.logStates[stepKey(view.selection)] ?? EMPTY_LOG_STATE
    : EMPTY_LOG_STATE

  return {
    ...view,
    performAction,
    refreshDetail,
    refreshLogs,
    selectedLogState,
    toggleAttempt,
    toggleJob,
    toggleStep,
  }
}

function stepKey(selection: StepSelection) {
  return `${selection.jobKey}:${selection.attemptId}:${selection.stepIndex}`
}

function sameSelection(
  left: StepSelection | null,
  right: StepSelection | null,
) {
  return left?.attemptId === right?.attemptId &&
    left?.jobKey === right?.jobKey &&
    left?.stepIndex === right?.stepIndex
}

function selectionExists(
  selection: StepSelection,
  jobs: readonly RepoRunJobDetail[],
) {
  return jobs.some(({ job, attempts }) =>
    job.key === selection.jobKey &&
    attempts.some((attempt) =>
      attempt.id === selection.attemptId &&
      attempt.steps.some((step) => step.index === selection.stepIndex)
    )
  )
}

function attemptJobKey(
  jobs: readonly RepoRunJobDetail[],
  attemptId: string,
) {
  return jobs.find(({ attempts }) =>
    attempts.some((attempt) => attempt.id === attemptId)
  )?.job.key ?? null
}

function withBoundedLogStates(
  states: Record<string, StepLogState>,
  key: string,
  value: StepLogState,
) {
  const next = { ...states }
  delete next[key]
  next[key] = value
  const keys = Object.keys(next)
  for (const staleKey of keys.slice(0, -MAX_CACHED_LOG_STEPS)) {
    delete next[staleKey]
  }
  return next
}

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : 'Run operation failed.'
}
