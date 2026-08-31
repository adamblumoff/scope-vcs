import {
  HttpError,
  InvalidApiResponseError,
  throwApiResponseError,
  validationIssuePath,
} from '../../api/http'
import type { RepoLiveState } from '../../api/types'
import type { ErrorResponse, RepoChangeEvent } from '../../api/types.generated'
import { apiValidators } from '../../api/validators.generated'

const INITIAL_RECONNECT_DELAY_MS = 2_000
const MAX_RECONNECT_DELAY_MS = 30_000

export type RepoStreamParseOutcome =
  | { type: 'event'; event: RepoChangeEvent }
  | { type: 'ignored' }
  | { type: 'stream-error'; error: ErrorResponse }
  | {
      type: 'protocol-error'
      failureClass: 'content-type' | 'json-syntax' | 'schema'
      issuePath?: string
    }

export type RepoStreamEnd = Exclude<
  RepoStreamParseOutcome,
  { type: 'event' } | { type: 'ignored' }
> | { type: 'transport' }

export type RepoStreamDiagnostic =
  | {
      type: 'protocol-error'
      failureClass: 'content-type' | 'json-syntax' | 'schema'
      issuePath?: string
    }
  | { type: 'transport'; consecutiveFailures: 3 }

type AuthTokenGetter = (options: { template: string }) => Promise<string | null>
type RepoEventStreamConnection = Pick<
  RepoLiveState,
  'clerk_token_template' | 'event_stream_url'
>
type ConnectRepoStream = (
  onEvent: (event: RepoChangeEvent) => void,
  signal: AbortSignal,
) => Promise<RepoStreamEnd>
type Wait = (milliseconds: number, signal: AbortSignal) => Promise<void>

export async function runRepoEventStream({
  connect,
  onDiagnostic = logRepoStreamDiagnostic,
  onEvent,
  onInterrupted,
  random = Math.random,
  signal,
  wait = abortableDelay,
}: {
  connect: ConnectRepoStream
  onDiagnostic?: (diagnostic: RepoStreamDiagnostic) => void
  onEvent: (event: RepoChangeEvent) => void
  onInterrupted: () => void
  random?: () => number
  signal: AbortSignal
  wait?: Wait
}) {
  let reconnectAttempt = 0
  let consecutiveTransportFailures = 0
  const deliver = (event: RepoChangeEvent) => {
    reconnectAttempt = 0
    consecutiveTransportFailures = 0
    onEvent(event)
  }

  while (!signal.aborted) {
    let outcome: RepoStreamEnd
    try {
      outcome = await connect(deliver, signal)
    } catch {
      outcome = { type: 'transport' }
    }
    if (signal.aborted) return

    onInterrupted()
    if (outcome.type === 'stream-error' && !outcome.error.retryable) return

    if (outcome.type === 'protocol-error') {
      consecutiveTransportFailures = 0
      onDiagnostic(outcome)
    } else if (outcome.type === 'transport') {
      consecutiveTransportFailures += 1
      if (consecutiveTransportFailures === 3) {
        onDiagnostic({ type: 'transport', consecutiveFailures: 3 })
      }
    } else {
      consecutiveTransportFailures = 0
    }

    const milliseconds = reconnectDelay(reconnectAttempt, random())
    reconnectAttempt += 1
    await wait(milliseconds, signal)
  }
}

export async function streamRepoEvents(
  live: RepoEventStreamConnection,
  getToken: AuthTokenGetter,
  onEvent: (event: RepoChangeEvent) => void,
  signal: AbortSignal,
): Promise<RepoStreamEnd> {
  const token = await getToken({ template: live.clerk_token_template })
  const headers = new Headers()
  if (token) headers.set('authorization', `Bearer ${token}`)
  const init = { headers, signal }
  const response = await fetch(live.event_stream_url, init)
  if (!response.ok) {
    try {
      await throwApiResponseError(response, live.event_stream_url, init)
    } catch (error) {
      if (error instanceof HttpError) {
        return { type: 'stream-error', error: error.response }
      }
      if (error instanceof InvalidApiResponseError) {
        return {
          type: 'protocol-error',
          failureClass: error.failureClass === 'content-type'
            ? 'content-type'
            : error.failureClass === 'schema'
              ? 'schema'
              : 'json-syntax',
          issuePath: error.issuePath,
        }
      }
      throw error
    }
  }
  if (!response.body) return { type: 'transport' }
  if (!isEventStream(response.headers.get('content-type'))) {
    await cancelBody(response.body)
    return { type: 'protocol-error', failureClass: 'content-type' }
  }

  const reader = response.body.getReader()
  const decoder = new TextDecoder()
  let buffer = ''
  try {
    while (!signal.aborted) {
      const chunk = await reader.read()
      if (chunk.done) return { type: 'transport' }
      buffer += decoder.decode(chunk.value, { stream: true })
      buffer = normalizeSseLineEndings(buffer)
      const taken = takeSseMessages(buffer)
      buffer = taken.rest
      for (const message of taken.messages) {
        const outcome = parseRepoStreamMessage(message)
        if (outcome.type === 'event') onEvent(outcome.event)
        else if (outcome.type !== 'ignored') return outcome
      }
    }
    return { type: 'transport' }
  } catch {
    return { type: 'transport' }
  } finally {
    try {
      await reader.cancel()
    } catch {
      // The fetch abort may have already errored the stream.
    }
    reader.releaseLock()
  }
}

export function parseRepoStreamMessage(message: string): RepoStreamParseOutcome {
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

  if (eventName !== 'repo-change' && eventName !== 'error') {
    return { type: 'ignored' }
  }
  if (data.length === 0) {
    return { type: 'protocol-error', failureClass: 'schema', issuePath: '/data' }
  }

  let payload: unknown
  try {
    payload = JSON.parse(data.join('\n'))
  } catch {
    return { type: 'protocol-error', failureClass: 'json-syntax', issuePath: '/data' }
  }

  if (eventName === 'repo-change') {
    if (!apiValidators.RepoChangeEvent(payload)) {
      return {
        type: 'protocol-error',
        failureClass: 'schema',
        issuePath: validationIssuePath(apiValidators.RepoChangeEvent.errors?.[0]),
      }
    }
    return { type: 'event', event: payload }
  }
  if (!apiValidators.ErrorResponse(payload)) {
    return {
      type: 'protocol-error',
      failureClass: 'schema',
      issuePath: validationIssuePath(apiValidators.ErrorResponse.errors?.[0]),
    }
  }
  return { type: 'stream-error', error: payload }
}

export function reconnectDelay(attempt: number, jitter: number) {
  const exponent = Math.min(Math.max(0, attempt), 4)
  const base = Math.min(
    INITIAL_RECONNECT_DELAY_MS * (2 ** exponent),
    MAX_RECONNECT_DELAY_MS,
  )
  const boundedJitter = Math.min(1, Math.max(0, jitter))
  return Math.min(
    MAX_RECONNECT_DELAY_MS,
    Math.round(base + base * 0.25 * boundedJitter),
  )
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

function isEventStream(contentType: string | null) {
  return contentType?.split(';', 1)[0]?.trim().toLowerCase() === 'text/event-stream'
}

async function cancelBody(body: ReadableStream<Uint8Array>) {
  try {
    await body.cancel()
  } catch {
    // A transport failure can error the body before it is rejected here.
  }
}

function normalizeSseLineEndings(buffer: string) {
  return buffer.replace(/\r\n/g, '\n').replace(/\r(?!$)/g, '\n')
}

function abortableDelay(milliseconds: number, signal: AbortSignal) {
  return new Promise<void>((resolve) => {
    if (signal.aborted) {
      resolve()
      return
    }
    const timeout = window.setTimeout(resolve, milliseconds)
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

function logRepoStreamDiagnostic(diagnostic: RepoStreamDiagnostic) {
  console.warn('[scope] repository event stream failure', diagnostic)
}
