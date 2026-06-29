import type { RepoLiveState } from '@/api/types'
import { useAuth } from '@clerk/tanstack-react-start'
import { useEffect } from 'react'

const RECONNECT_DELAY_MS = 2000

type RepoChangeEvent = {
  repo_id: string
  version: number
  reason: string
}

type AuthTokenGetter = (options: { template: string }) => Promise<string | null>
type StreamRepoEventsResult = 'closed'

export function useRepoLiveRefresh(
  live: RepoLiveState | null,
  invalidate: () => Promise<unknown>,
) {
  const { getToken, isLoaded } = useAuth()

  useEffect(() => {
    if (!live || !isLoaded || !canUseRepoLiveRefresh(live)) {
      return
    }

    const controller = new AbortController()
    let stopped = false
    let highestAppliedVersion = live.repo.change_version
    let forceRefreshPending = false
    let pendingVersion: number | null = null
    let refreshInFlight = false
    let retryTimeout: number | null = null

    const scheduleRetry = () => {
      if (stopped || retryTimeout !== null) {
        return
      }
      retryTimeout = window.setTimeout(() => {
        retryTimeout = null
        void flushRefresh()
      }, RECONNECT_DELAY_MS)
    }

    const flushRefresh = async () => {
      if (
        stopped ||
        refreshInFlight ||
        (pendingVersion === null && !forceRefreshPending)
      ) {
        return
      }

      const version = pendingVersion
      const forceRefresh = forceRefreshPending
      pendingVersion = null
      forceRefreshPending = false
      refreshInFlight = true
      let shouldRetry = false
      try {
        await invalidate()
        if (version !== null) {
          highestAppliedVersion = Math.max(highestAppliedVersion, version)
        }
      } catch (error) {
        if (version !== null) {
          pendingVersion = Math.max(pendingVersion ?? version, version)
        }
        forceRefreshPending ||= forceRefresh
        shouldRetry = true
      } finally {
        refreshInFlight = false
        if (stopped || (pendingVersion === null && !forceRefreshPending)) {
          return
        }
        if (shouldRetry) {
          scheduleRetry()
        } else {
          void flushRefresh()
        }
      }
    }

    const onEvent = (event: RepoChangeEvent) => {
      if (event.repo_id !== live.repo.id) {
        return
      }
      if (event.reason === 'connected') {
        return
      }
      if (event.reason === 'lagged') {
        forceRefreshPending = true
        void flushRefresh()
        return
      }
      if (!usesVersionedRepoChangeEvents(live)) {
        forceRefreshPending = true
        void flushRefresh()
        return
      }
      if (event.version <= highestAppliedVersion) {
        return
      }
      pendingVersion = Math.max(pendingVersion ?? event.version, event.version)
      void flushRefresh()
    }

    const onStreamInterrupted = () => {
      forceRefreshPending = true
      void flushRefresh()
    }

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
      if (retryTimeout !== null) {
        window.clearTimeout(retryTimeout)
      }
      controller.abort()
    }
  }, [getToken, invalidate, isLoaded, live])
}

export function canUseRepoLiveRefresh(_live: RepoLiveState) {
  return true
}

export function usesVersionedRepoChangeEvents(live: RepoLiveState) {
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

  const payload = JSON.parse(data.join('\n')) as Partial<RepoChangeEvent>
  if (
    typeof payload.repo_id !== 'string' ||
    typeof payload.version !== 'number' ||
    typeof payload.reason !== 'string'
  ) {
    return null
  }

  return {
    reason: payload.reason,
    repo_id: payload.repo_id,
    version: payload.version,
  }
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
