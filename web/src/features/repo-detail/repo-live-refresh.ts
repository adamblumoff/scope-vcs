import type { RepoLiveState } from '@/api/types'
import type { RepoChangeEvent } from '@/api/types.generated'
import { useAuth } from '@clerk/tanstack-react-start'
import { useCallback, useEffect, useRef } from 'react'
import { runRepoEventStream, streamRepoEvents } from './repo-event-stream'

const REFRESH_RETRY_DELAY_MS = 2_000

type RetryScheduler = (retry: () => void) => () => void
export type RepoChangeListener = (event: RepoChangeEvent) => void
export type SubscribeToRepoChanges = (
  listener: RepoChangeListener,
) => () => void

export type RepoRefreshCoordinator = {
  onEvent: (event: RepoChangeEvent) => void
  onStreamInterrupted: () => void
  stop: () => void
}

export function useRepoLiveRefresh(
  live: RepoLiveState | null,
  invalidate: () => Promise<unknown>,
) {
  const { getToken, isLoaded } = useAuth()
  const listenersRef = useRef(new Set<RepoChangeListener>())
  const subscribe = useCallback<SubscribeToRepoChanges>((listener) => {
    listenersRef.current.add(listener)
    return () => listenersRef.current.delete(listener)
  }, [])

  useEffect(() => {
    if (!live || !isLoaded) {
      return
    }

    const controller = new AbortController()
    const coordinator = createRepoRefreshCoordinator({
      initialVersion: live.repo.change_version,
      invalidate,
      repoId: live.repo.id,
      scheduleRetry: browserRetryScheduler,
      versioned: usesVersionedRepoChangeEvents(live),
    })
    const notifyListeners = (event: RepoChangeEvent) => {
      for (const listener of listenersRef.current) {
        try {
          listener(event)
        } catch {
          // A broken page subscriber must not tear down the shared stream.
        }
      }
    }
    const onEvent = (event: RepoChangeEvent) => {
      coordinator.onEvent(event)
      notifyListeners(event)
    }
    const onStreamInterrupted = () => {
      coordinator.onStreamInterrupted()
      const event: RepoChangeEvent = {
        incarnation_id: 'local-stream-interruption',
        kind: 'Lagged',
        repo_id: live.repo.id,
        version: 0,
      }
      notifyListeners(event)
    }

    void runRepoEventStream({
      connect: (deliver, signal) =>
        streamRepoEvents(live, getToken, deliver, signal),
      onEvent,
      onInterrupted: onStreamInterrupted,
      signal: controller.signal,
    })
    return () => {
      coordinator.stop()
      controller.abort()
    }
  }, [getToken, invalidate, isLoaded, live])

  return subscribe
}

export function createRepoRefreshCoordinator({
  initialVersion,
  invalidate,
  repoId,
  scheduleRetry,
  versioned,
}: {
  initialVersion: number
  invalidate: () => Promise<unknown>
  repoId: string
  scheduleRetry: RetryScheduler
  versioned: boolean
}): RepoRefreshCoordinator {
  let stopped = false
  let highestAppliedVersion = initialVersion
  let forceRefreshPending = false
  let pendingVersion: number | null = null
  let refreshInFlight = false
  let cancelRetry: (() => void) | null = null

  const flushRefresh = async () => {
    if (stopped || refreshInFlight || (pendingVersion === null && !forceRefreshPending)) return

    const version = pendingVersion
    const forceRefresh = forceRefreshPending
    pendingVersion = null
    forceRefreshPending = false
    refreshInFlight = true
    try {
      await invalidate()
      if (version !== null) {
        highestAppliedVersion = Math.max(highestAppliedVersion, version)
        if (pendingVersion !== null && pendingVersion <= highestAppliedVersion) {
          pendingVersion = null
        }
      }
    } catch {
      if (version !== null) pendingVersion = Math.max(pendingVersion ?? version, version)
      forceRefreshPending ||= forceRefresh
      if (!stopped && cancelRetry === null) {
        cancelRetry = scheduleRetry(() => {
          cancelRetry = null
          void flushRefresh()
        })
      }
      return
    } finally {
      refreshInFlight = false
    }
    if (!stopped && (pendingVersion !== null || forceRefreshPending)) void flushRefresh()
  }

  const requestRefresh = (version: number | null) => {
    if (version === null) forceRefreshPending = true
    else pendingVersion = Math.max(pendingVersion ?? version, version)
    void flushRefresh()
  }

  return {
    onEvent(event) {
      if (
        stopped ||
        event.repo_id !== repoId ||
        event.kind === 'Connected' ||
        typeof event.kind === 'object' &&
          ('RequestTimelineChanged' in event.kind || 'RunChanged' in event.kind)
      ) {
        return
      }
      if (event.kind === 'Lagged' || !versioned || event.version === 0) {
        requestRefresh(null)
      } else if (event.version > highestAppliedVersion) {
        requestRefresh(event.version)
      }
    },
    onStreamInterrupted() {
      if (!stopped) requestRefresh(null)
    },
    stop() {
      stopped = true
      cancelRetry?.()
      cancelRetry = null
    },
  }
}

function browserRetryScheduler(retry: () => void) {
  const timeout = window.setTimeout(retry, REFRESH_RETRY_DELAY_MS)
  return () => window.clearTimeout(timeout)
}

function usesVersionedRepoChangeEvents(live: RepoLiveState) {
  return live.repo.access.actor !== 'Public'
}
