import type { RepoLiveState } from '@/api/types'
import type { RepoChangeEvent } from '@/api/types.generated'
import { useAuth } from '@clerk/tanstack-react-start'
import { useCallback, useEffect, useRef } from 'react'

const RECONNECT_DELAY_MS = 2000

type AuthTokenGetter = (options: { template: string }) => Promise<string | null>
type StreamRepoEventsResult = 'closed'
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
        kind: 'Lagged',
        repo_id: live.repo.id,
        version: 0,
      }
      notifyListeners(event)
    }

    let stopped = false
    const run = async () => {
      while (!stopped) {
        try {
          const result = await streamRepoEvents(
            live,
            getToken,
            onEvent,
            controller.signal,
          )
          if (!controller.signal.aborted && result === 'closed') {
            onStreamInterrupted()
          }
        } catch (error) {
          if (controller.signal.aborted) {
            return
          }
          onStreamInterrupted()
        }
        if (!stopped) {
          await delay(RECONNECT_DELAY_MS, controller.signal)
        }
      }
    }

    void run()
    return () => {
      stopped = true
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
          'RequestTimelineChanged' in event.kind
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
  const timeout = window.setTimeout(retry, RECONNECT_DELAY_MS)
  return () => window.clearTimeout(timeout)
}

function usesVersionedRepoChangeEvents(live: RepoLiveState) {
  return live.repo.access.actor !== 'Public'
}

async function streamRepoEvents(
  live: RepoLiveState,
  getToken: AuthTokenGetter,
  onEvent: (event: RepoChangeEvent) => void,
  signal: AbortSignal,
): Promise<StreamRepoEventsResult> {
  const token = await getToken({ template: live.clerk_token_template })
  const headers = new Headers()
  if (token) {
    headers.set('authorization', `Bearer ${token}`)
  }

  const response = await fetch(live.event_stream_url, { headers, signal })
  if (!response.ok || !response.body) {
    return 'closed'
  }

  const reader = response.body.getReader()
  const decoder = new TextDecoder()
  let buffer = ''
  while (!signal.aborted) {
    const chunk = await reader.read()
    if (chunk.done) {
      return 'closed'
    }
    buffer += decoder.decode(chunk.value, { stream: true }).replace(/\r\n/g, '\n')
    const taken = takeSseMessages(buffer)
    buffer = taken.rest
    for (const message of taken.messages) {
      const event = parseRepoChangeEvent(message)
      if (event) {
        onEvent(event)
      }
    }
  }
  return 'closed'
}

export function parseRepoChangeEvent(message: string): RepoChangeEvent | null {
  const lines = message.split('\n')
  let eventName = ''
  const data: string[] = []
  for (const line of lines) {
    if (line.startsWith('event:')) {
      eventName = line.slice('event:'.length).trim()
    } else if (line.startsWith('data:')) {
      data.push(line.slice('data:'.length).trimStart())
    }
  }

  if (eventName !== 'repo-change' || data.length === 0) {
    return null
  }

  let payload: Partial<RepoChangeEvent>
  try {
    payload = JSON.parse(data.join('\n')) as Partial<RepoChangeEvent>
  } catch {
    return null
  }
  if (
    typeof payload.repo_id !== 'string' ||
    typeof payload.version !== 'number' ||
    !isRepoChangeKind(payload.kind)
  ) {
    return null
  }

  return {
    kind: payload.kind,
    repo_id: payload.repo_id,
    version: payload.version,
  }
}

function isRepoChangeKind(value: unknown): value is RepoChangeEvent['kind'] {
  if (value === 'Connected' || value === 'Lagged') return true
  if (!value || typeof value !== 'object') return false
  if ('RepositoryChanged' in value) {
    const changed = value.RepositoryChanged
    return (
      !!changed &&
      typeof changed === 'object' &&
      'reason' in changed &&
      typeof changed.reason === 'string'
    )
  }
  if ('RequestTimelineChanged' in value) {
    const changed = value.RequestTimelineChanged
    return (
      !!changed &&
      typeof changed === 'object' &&
      'request_id' in changed &&
      typeof changed.request_id === 'string' &&
      'discussion_id' in changed &&
      typeof changed.discussion_id === 'string' &&
      'through_position' in changed &&
      typeof changed.through_position === 'number'
    )
  }
  return false
}

export function takeSseMessages(buffer: string) {
  const messages: string[] = []
  let rest = buffer
  let separator = rest.indexOf('\n\n')
  while (separator >= 0) {
    messages.push(rest.slice(0, separator))
    rest = rest.slice(separator + 2)
    separator = rest.indexOf('\n\n')
  }
  return { messages, rest }
}

function delay(ms: number, signal: AbortSignal) {
  return new Promise<void>((resolve) => {
    if (signal.aborted) {
      resolve()
      return
    }
    const timeout = window.setTimeout(resolve, ms)
    signal.addEventListener(
      'abort',
      () => {
        window.clearTimeout(timeout)
        resolve()
      },
      { once: true },
    )
  })
}
