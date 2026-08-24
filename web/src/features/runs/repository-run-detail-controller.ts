import type {
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
  defaultShowGraph,
  mergeStepLogs,
  reconcileAttemptOverrides,
  selectAttempt as selectAttemptInJob,
  selectJob,
  selectStep,
  runCanChange,
  selectInitialStep,
  type StepSelection,
} from './repository-run-detail-model'
import { useRunLiveRefresh, type RunRefresh } from './run-live-refresh'

export type { StepSelection } from './repository-run-detail-model'

const MAX_CACHED_LOG_STEPS = 8
const DETAIL_CHANGES = ['StatusChanged', 'LogsAppended'] as const

export type StepLogState = {
  error: string | null
  loading: boolean
  logs: RepoRunLog[]
  logsTruncated: boolean
  nextAfter: number
}

type DetailViewState = {
  actionError: string | null
  attemptOverrides: Record<string, string>
  detail: RepoRunDetail
  logStates: Record<string, StepLogState>
  manualSelection: boolean
  metadataError: string | null
  pendingAction: 'cancel' | 'retry' | null
  reconciliationGeneration: number | null
  selectedJobKey: string | null
  selection: StepSelection | null
  showGraph: boolean
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
  const initialSelection = selectInitialStep(detail.jobs)
  return {
    actionError: null,
    attemptOverrides: {},
    detail,
    logStates: {},
    manualSelection: false,
    metadataError: null,
    pendingAction: null,
    reconciliationGeneration: null,
    selectedJobKey: initialSelection?.jobKey ?? null,
    selection: initialSelection,
    showGraph: defaultShowGraph(detail.jobs),
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
  loadDetail: (signal?: AbortSignal) => Promise<RepoRunDetail>
  loadLogs: (
    input: RunStepLogsInput,
    signal?: AbortSignal,
  ) => Promise<RepoRunStepLogPage>
  params: RunActionInput
}) {
  const [view, updateView] = useReducer(
    updateDetailView,
    initialDetail,
    createDetailViewState,
  )
  const detailInFlightRef = useRef<Promise<void> | null>(null)
  const detailGenerationRef = useRef(0)
  const logInFlightRef = useRef<Map<string, Promise<boolean>> | null>(null)
  if (logInFlightRef.current === null) {
    logInFlightRef.current = new Map()
  }
  const logStatesRef = useRef(view.logStates)
  const selectionRef = useRef(view.selection)
  const mountedRef = useRef(false)

  useEffect(() => {
    logStatesRef.current = view.logStates
  }, [view.logStates])

  useEffect(() => {
    selectionRef.current = view.selection
  }, [view.selection])

  const refreshDetail = useCallback(async (
    forceAfterInFlight = false,
    signal?: AbortSignal,
  ) => {
    if (detailInFlightRef.current) {
      await detailInFlightRef.current
      if (!forceAfterInFlight) return
    }
    const generation = ++detailGenerationRef.current
    const request = loadDetail(signal)
      .then((nextDetail) => {
        if (!mountedRef.current) return
        updateView((current) => {
          const reconciledAction = current.reconciliationGeneration !== null &&
            generation >= current.reconciliationGeneration
          const selectionStillValid = current.selection !== null &&
            selectionExists(current.selection, nextDetail.jobs)
          const nextSelection = selectionStillValid
            ? current.selection
            : current.manualSelection
              ? null
              : selectInitialStep(nextDetail.jobs)
          const selectedJobKey = nextSelection
            ? nextSelection.jobKey
            : jobExists(current.selectedJobKey, nextDetail.jobs)
              ? current.selectedJobKey
              : null
          return {
            ...current,
            attemptOverrides: reconcileAttemptOverrides(
              current.attemptOverrides,
              nextDetail.jobs,
            ),
            detail: nextDetail,
            metadataError: null,
            pendingAction: reconciledAction ? null : current.pendingAction,
            reconciliationGeneration: reconciledAction
              ? null
              : current.reconciliationGeneration,
            selectedJobKey,
            selection: nextSelection,
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
        throw error
      })
      .finally(() => {
        if (detailInFlightRef.current === request) {
          detailInFlightRef.current = null
        }
      })
    detailInFlightRef.current = request
    return request
  }, [loadDetail])

  const refreshLogs = useCallback((
    target: StepSelection,
    signal?: AbortSignal,
  ) => {
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
    const request = loadLogs(
      {
        ...params,
        after: current.nextAfter,
        attempt_id: target.attemptId,
        step_index: target.stepIndex,
      },
      signal,
    )
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
    signal?: AbortSignal,
  ) => {
    const key = stepKey(target)
    const existing = logInFlightRef.current?.get(key)
    if (existing) await existing
    let previousAfter: number
    do {
      previousAfter = logStatesRef.current[key]?.nextAfter ?? 0
      if (!await refreshLogs(target, signal)) return false
    } while (
      mountedRef.current &&
      (logStatesRef.current[key]?.nextAfter ?? 0) > previousAfter
    )
    return mountedRef.current
  }, [refreshLogs])

  const refreshFromRunEvents = useCallback<RunRefresh>(async (
    reasons,
    signal,
  ) => {
    const refreshMetadata = reasons.has('Recovery') ||
      reasons.has('StatusChanged')
    if (refreshMetadata) await refreshDetail(false, signal)
    const selection = selectionRef.current
    if (selection && (refreshMetadata || reasons.has('LogsAppended'))) {
      if (!await refreshLogsAfterInFlight(selection, signal)) {
        throw new Error('Selected run logs could not refresh.')
      }
    }
  }, [refreshDetail, refreshLogsAfterInFlight])

  const refreshRun = useRunLiveRefresh({
    acceptedChanges: DETAIL_CHANGES,
    mutable: runCanChange(view.detail.run.state),
    refresh: refreshFromRunEvents,
    runId: params.run_id,
  })

  useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
    }
  }, [])

  useEffect(() => {
    const selection = view.selection
    if (!selection) return
    if (!runCanChange(view.detail.run.state)) {
      void refreshLogsAfterInFlight(selection)
      return
    }
    void refreshLogs(selection)
  }, [
    refreshLogs,
    refreshLogsAfterInFlight,
    view.detail.run.state,
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

  // Navigation rules live in the model so `selection` and `selectedJobKey`
  // cannot drift apart here.
  function toggleJob(jobDetail: RepoRunJobDetail) {
    updateView((current) => selectJob(current, jobDetail.job.key))
  }

  function selectAttempt(jobKey: string, attemptId: string) {
    updateView((current) => selectAttemptInJob(current, jobKey, attemptId))
  }

  function toggleStep(jobKey: string, attemptId: string, stepIndex: number) {
    updateView((current) => selectStep(current, { attemptId, jobKey, stepIndex }))
  }

  function toggleGraph() {
    updateView((current) => ({ ...current, showGraph: !current.showGraph }))
  }

  const selectedLogState = view.selection
    ? view.logStates[stepKey(view.selection)] ?? EMPTY_LOG_STATE
    : EMPTY_LOG_STATE

  return {
    ...view,
    performAction,
    refreshDetail: refreshRun,
    refreshLogs,
    selectAttempt,
    selectedLogState,
    toggleGraph,
    toggleJob,
    toggleStep,
  }
}

function stepKey(selection: StepSelection) {
  return `${selection.jobKey}:${selection.attemptId}:${selection.stepIndex}`
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

function jobExists(jobKey: string | null, jobs: readonly RepoRunJobDetail[]) {
  return jobKey !== null && jobs.some(({ job }) => job.key === jobKey)
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
