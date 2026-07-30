import type {
  RepoRunAttempt,
  RepoRunDetail,
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
  mergeStepLogs,
  reconcileAutomaticStepSelection,
  reconcileExpandedAttempts,
} from './repository-run-detail-model'

const REFRESH_INTERVAL_MS = 2_000
const MAX_CACHED_LOG_STEPS = 8

export type StepSelection = {
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
  const attemptIds = detail.attempts.map((attempt) => attempt.id)
  const initialAttempt = detail.attempts[0]
  const initialStepIndex = initialAttempt
    ? defaultSelectedStep(initialAttempt.steps)
    : null
  return {
    actionError: null,
    detail,
    expandedAttempts: reconcileExpandedAttempts(new Set(), [], attemptIds),
    logStates: {},
    metadataError: null,
    pendingAction: null,
    pendingAutomaticSelection: null,
    reconciliationGeneration: null,
    selection: initialAttempt && initialStepIndex !== null
      ? { attemptId: initialAttempt.id, stepIndex: initialStepIndex }
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
    knownAttemptIdsRef.current = initialDetail.attempts.map(
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
        const nextIds = nextDetail.attempts.map((attempt) => attempt.id)
        const previousIds = knownAttemptIdsRef.current ?? []
        const newestIsNew = nextIds[0] !== undefined &&
          !previousIds.includes(nextIds[0])
        knownAttemptIdsRef.current = nextIds
        updateView((current) => {
          let nextSelection = current.selection
          let pendingAutomaticSelection = current.pendingAutomaticSelection
          let selectionIsAutomatic = current.selectionIsAutomatic
          const newest = nextDetail.attempts[0]
          const previousNewest = current.detail.attempts[0]
          const newestGainedSteps = newest?.id === previousNewest?.id &&
            previousNewest.steps.length === 0 &&
            newest.steps.length > 0
          if (
            current.selectionIsAutomatic &&
            (newestIsNew || newestGainedSteps) &&
            newest
          ) {
            const stepIndex = defaultSelectedStep(newest.steps)
            if (stepIndex !== null) {
              const candidate = { attemptId: newest.id, stepIndex }
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
                nextDetail.attempts,
              )
            ) {
              // Keep draining the prior selection before the pending handoff.
            } else {
              const candidate = reconcileAutomaticStepSelection(
                current.selection,
                nextDetail.attempts,
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
            !selectionExists(nextSelection, nextDetail.attempts)
          ) {
            nextSelection = null
            pendingAutomaticSelection = null
          }
          const reconciledAction = current.reconciliationGeneration !== null &&
            generation >= current.reconciliationGeneration
          return {
            ...current,
            detail: nextDetail,
            expandedAttempts: reconcileExpandedAttempts(
              current.expandedAttempts,
              previousIds,
              nextIds,
            ),
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
    const timer = window.setInterval(
      () => void refreshDetail(),
      REFRESH_INTERVAL_MS,
    )
    return () => {
      mountedRef.current = false
      window.clearInterval(timer)
    }
  }, [refreshDetail])

  useEffect(() => {
    const selection = view.selection
    if (!selection) return
    void refreshLogs(selection)
    const timer = window.setInterval(
      () => void refreshLogs(selection),
      REFRESH_INTERVAL_MS,
    )
    return () => window.clearInterval(timer)
  }, [refreshLogs, view.selection])

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

  function toggleAttempt(attempt: RepoRunAttempt) {
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
          nextSelection = { attemptId: attempt.id, stepIndex }
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

  function toggleStep(attemptId: string, stepIndex: number) {
    updateView((current) => ({
      ...current,
      pendingAutomaticSelection: null,
      selection: current.selection?.attemptId === attemptId &&
        current.selection.stepIndex === stepIndex
        ? null
        : { attemptId, stepIndex },
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
    toggleStep,
  }
}

function stepKey(selection: StepSelection) {
  return `${selection.attemptId}:${selection.stepIndex}`
}

function sameSelection(
  left: StepSelection | null,
  right: StepSelection | null,
) {
  return left?.attemptId === right?.attemptId &&
    left?.stepIndex === right?.stepIndex
}

function selectionExists(
  selection: StepSelection,
  attempts: readonly RepoRunAttempt[],
) {
  return attempts.some((attempt) =>
    attempt.id === selection.attemptId &&
    attempt.steps.some((step) => step.index === selection.stepIndex)
  )
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
