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

export function useRepoLiveRefresh(
  live: RepoLiveState | null,
  invalidate: () => Promise<unknown>,
) {
  const { getToken, isLoaded } = useAuth()

  useEffect(() => {
    if (!live || !isLoaded) {
      return
    }

    const controller = new AbortController()
    let stopped = false
    let highestInvalidatedVersion = live.repo.change_version

    const onEvent = (event: RepoChangeEvent) => {
      if (
        event.repo_id !== live.repo.id ||
        event.version <= highestInvalidatedVersion
      ) {
        return
      }
      highestInvalidatedVersion = event.version
      void invalidate().catch(() => undefined)
    }

    const run = async () => {
      while (!stopped) {
        try {
          await streamRepoEvents(live, getToken, onEvent, controller.signal)
        } catch (error) {
          if (controller.signal.aborted) {
            return
          }
        }
        if (!stopped) {
          await delay(RECONNECT_DELAY_MS, controller.signal)
        }
      }
    }

    void run()
    return () => {
      stopped = true
      controller.abort()
    }
  }, [getToken, invalidate, isLoaded, live])
}

async function streamRepoEvents(
  live: RepoLiveState,
  getToken: AuthTokenGetter,
  onEvent: (event: RepoChangeEvent) => void,
  signal: AbortSignal,
) {
  const token = await getToken({ template: live.clerk_token_template })
  const headers = new Headers()
  if (token) {
    headers.set('authorization', `Bearer ${token}`)
  }

  const response = await fetch(live.event_stream_url, { headers, signal })
  if (!response.ok || !response.body) {
    return
  }

  const reader = response.body.getReader()
  const decoder = new TextDecoder()
  let buffer = ''
  while (!signal.aborted) {
    const chunk = await reader.read()
    if (chunk.done) {
      return
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
