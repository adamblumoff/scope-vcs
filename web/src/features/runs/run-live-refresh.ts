import type {
  RepoChangeEvent,
  RunChangeKind,
} from '@/api/types.generated'
import { useCallback, useEffect, useMemo, useRef } from 'react'
import {
  useRepoChangeSubscription,
  useRepoLayout,
} from '../repo-detail/repo-layout-context'

const RECONCILIATION_INTERVAL_MS = 30_000
const REFRESH_RETRY_DELAY_MS = 2_000
const REFRESH_TIMEOUT_MS = 15_000

export type RunRefreshReason = RunChangeKind | 'Recovery'
export type RunRefresh = (
  reasons: ReadonlySet<RunRefreshReason>,
  signal: AbortSignal,
) => Promise<unknown>

type Scheduler = (callback: () => void, delayMs: number) => () => void

export type RunRefreshCoordinator = {
  onEvent: (event: RepoChangeEvent) => void
  requestRefresh: (reason?: RunRefreshReason) => void
  stop: () => void
}

export function useRunLiveRefresh({
  acceptedChanges,
  mutable,
  refresh,
  runId,
}: {
  acceptedChanges: readonly RunChangeKind[]
  mutable: boolean
  refresh: RunRefresh
  runId?: string
}) {
  const { repo } = useRepoLayout()
  const refreshRef = useRef(refresh)
  const mutableRef = useRef(mutable)
  useEffect(() => {
    refreshRef.current = refresh
    mutableRef.current = mutable
  }, [mutable, refresh])

  const coordinator = useMemo(() => createRunRefreshCoordinator({
    acceptedChanges,
    refresh: (reasons, signal) => refreshRef.current(reasons, signal),
    repoId: repo.id,
    runId,
    schedule: browserScheduler,
  }), [acceptedChanges, repo.id, runId])

  const onEvent = useCallback((event: RepoChangeEvent) => {
    if (mutableRef.current) coordinator.onEvent(event)
  }, [coordinator])
  useRepoChangeSubscription(onEvent)

  useEffect(() => () => coordinator.stop(), [coordinator])

  useEffect(() => {
    if (!mutable) return
    const reconcile = () => coordinator.requestRefresh('Recovery')
    reconcile()
    const onFocus = () => {
      if (document.visibilityState === 'visible') reconcile()
    }
    window.addEventListener('focus', onFocus)
    window.addEventListener('online', reconcile)
    document.addEventListener('visibilitychange', onFocus)
    const interval = window.setInterval(
      reconcile,
      RECONCILIATION_INTERVAL_MS,
    )
    return () => {
      window.removeEventListener('focus', onFocus)
      window.removeEventListener('online', reconcile)
      document.removeEventListener('visibilitychange', onFocus)
      window.clearInterval(interval)
    }
  }, [coordinator, mutable])

  return useCallback(
    () => coordinator.requestRefresh('Recovery'),
    [coordinator],
  )
}

export function createRunRefreshCoordinator({
  acceptedChanges,
  refresh,
  repoId,
  runId,
  schedule,
  timeoutMs = REFRESH_TIMEOUT_MS,
}: {
  acceptedChanges: readonly RunChangeKind[]
  refresh: RunRefresh
  repoId: string
  runId?: string
  schedule: Scheduler
  timeoutMs?: number
}): RunRefreshCoordinator {
  const accepted = new Set(acceptedChanges)
  let activeController: AbortController | null = null
  let cancelRetry: (() => void) | null = null
  let pending = new Set<RunRefreshReason>()
  let refreshInFlight = false
  let stopped = false

  const flush = async () => {
    if (stopped || refreshInFlight || pending.size === 0) return
    cancelRetry?.()
    cancelRetry = null
    const reasons = pending
    pending = new Set()
    refreshInFlight = true
    const controller = new AbortController()
    activeController = controller
    const cancelTimeout = schedule(() => controller.abort(), timeoutMs)
    const aborted = new Promise<never>((_resolve, reject) => {
      controller.signal.addEventListener(
        'abort',
        () => reject(new Error('Run refresh timed out.')),
        { once: true },
      )
    })
    let failed = false
    try {
      await Promise.race([
        refresh(reasons, controller.signal),
        aborted,
      ])
    } catch {
      failed = true
      if (!stopped) {
        for (const reason of reasons) pending.add(reason)
      }
    } finally {
      cancelTimeout()
      if (activeController === controller) activeController = null
      refreshInFlight = false
    }
    if (stopped || pending.size === 0) return
    if (failed) {
      cancelRetry = schedule(() => {
        cancelRetry = null
        void flush()
      }, REFRESH_RETRY_DELAY_MS)
    } else {
      void flush()
    }
  }

  const requestRefresh = (reason: RunRefreshReason = 'Recovery') => {
    if (stopped) return
    pending.add(reason)
    if (cancelRetry) {
      cancelRetry()
      cancelRetry = null
    }
    void flush()
  }

  return {
    onEvent(event) {
      if (stopped || event.repo_id !== repoId) return
      if (event.kind === 'Connected' || event.kind === 'Lagged') {
        requestRefresh('Recovery')
        return
      }
      if (
        typeof event.kind !== 'object' ||
        !('RunChanged' in event.kind)
      ) return
      const changed = event.kind.RunChanged
      if (
        (runId === undefined || changed.run_id === runId) &&
        accepted.has(changed.change)
      ) requestRefresh(changed.change)
    },
    requestRefresh,
    stop() {
      stopped = true
      pending.clear()
      cancelRetry?.()
      cancelRetry = null
      activeController?.abort()
      activeController = null
    },
  }
}

function browserScheduler(callback: () => void, delayMs: number) {
  const timeout = window.setTimeout(callback, delayMs)
  return () => window.clearTimeout(timeout)
}
