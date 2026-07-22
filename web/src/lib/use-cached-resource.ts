import { useCallback, useEffect, useState } from 'react'

type CachedResourceState<T extends object> =
  | { error: null; identity: null; status: 'idle'; value: null }
  | { error: null; identity: string; status: 'loading'; value: null }
  | { error: null; identity: string; status: 'loaded'; value: T }
  | { error: string; identity: string; status: 'failed'; value: null }

export type CachedResource<T extends object> = CachedResourceState<T> & {
  retry: () => void
}

const idleResource = {
  error: null,
  identity: null,
  status: 'idle',
  value: null,
} as const

export function useCachedResource<T extends object>({
  fallbackError,
  identity,
  load,
  peek,
  read,
  write,
}: {
  fallbackError: string
  identity: string | null
  load: (signal: AbortSignal) => Promise<T>
  peek: (identity: string) => T | null
  read: (identity: string) => T | null
  write: (identity: string, value: T) => void
}): CachedResource<T> {
  const cached = identity ? peek(identity) : null
  const [retryVersion, setRetryVersion] = useState(0)
  const [resource, setResource] = useState<CachedResourceState<T>>(() =>
    cachedResourceFor(identity, cached),
  )
  const visibleResource = resource.identity === identity
    ? resource
    : cachedResourceFor(identity, cached)

  useEffect(() => {
    if (!identity) {
      setResource(idleResource)
      return
    }

    const cachedValue = read(identity)
    if (cachedValue !== null) {
      setResource({
        error: null,
        identity,
        status: 'loaded',
        value: cachedValue,
      })
      return
    }

    setResource({ error: null, identity, status: 'loading', value: null })
    return startAbortableResourceAttempt({
      load,
      onFailed: (error) => {
        setResource({
          error: resourceErrorMessage(error, fallbackError),
          identity,
          status: 'failed',
          value: null,
        })
      },
      onLoaded: (value) => {
        write(identity, value)
        setResource({ error: null, identity, status: 'loaded', value })
      },
    })
  }, [fallbackError, identity, load, read, retryVersion, write])

  const retry = useCallback(() => setRetryVersion((version) => version + 1), [])
  return { ...visibleResource, retry }
}

export function cachedResourceFor<T extends object>(
  identity: string | null,
  cached: T | null,
): CachedResourceState<T> {
  if (!identity) return idleResource
  if (cached !== null) {
    return { error: null, identity, status: 'loaded', value: cached }
  }
  return { error: null, identity, status: 'loading', value: null }
}

export function startAbortableResourceAttempt<T>({
  load,
  onFailed,
  onLoaded,
}: {
  load: (signal: AbortSignal) => Promise<T>
  onFailed: (error: unknown) => void
  onLoaded: (value: T) => void
}) {
  const controller = new AbortController()
  let active = true

  void load(controller.signal).then(
    (value) => {
      if (!active || controller.signal.aborted) return
      onLoaded(value)
    },
    (error: unknown) => {
      if (!active || controller.signal.aborted) return
      onFailed(error)
    },
  )

  return () => {
    active = false
    controller.abort()
  }
}

export function resourceErrorMessage(error: unknown, fallback: string) {
  return error instanceof Error && error.message.trim()
    ? error.message
    : fallback
}
