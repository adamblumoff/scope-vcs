import { useCallback, useEffect, useState } from 'react'

type HistoryResourceState<T extends object> =
  | { error: null; identity: null; status: 'idle'; value: null }
  | { error: null; identity: string; status: 'loading'; value: null }
  | { error: null; identity: string; status: 'loaded'; value: T }
  | { error: string; identity: string; status: 'failed'; value: null }

export type HistoryResource<T extends object> = HistoryResourceState<T> & {
  retry: () => void
}

const idleResource = {
  error: null,
  identity: null,
  status: 'idle',
  value: null,
} as const

export function useHistoryResource<T extends object>({
  identity,
  load,
  read,
  write,
}: {
  identity: string | null
  load: (signal: AbortSignal) => Promise<T>
  read: (identity: string) => T | null
  write: (identity: string, value: T) => void
}): HistoryResource<T> {
  const cached = identity ? read(identity) : null
  const [retryVersion, setRetryVersion] = useState(0)
  const [resource, setResource] = useState<HistoryResourceState<T>>(() =>
    resourceFor(identity, cached),
  )
  const visibleResource = resource.identity === identity
    ? resource
    : resourceFor(identity, cached)

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

    const controller = new AbortController()
    let active = true
    setResource({ error: null, identity, status: 'loading', value: null })
    void load(controller.signal).then(
      (value) => {
        if (!active || controller.signal.aborted) return
        write(identity, value)
        setResource({ error: null, identity, status: 'loaded', value })
      },
      (error: unknown) => {
        if (!active || controller.signal.aborted) return
        setResource({
          error: error instanceof Error && error.message.trim()
            ? error.message
            : 'Resource is unavailable.',
          identity,
          status: 'failed',
          value: null,
        })
      },
    )

    return () => {
      active = false
      controller.abort()
    }
  }, [identity, load, read, retryVersion, write])

  const retry = useCallback(() => setRetryVersion((version) => version + 1), [])
  return { ...visibleResource, retry }
}

function resourceFor<T extends object>(
  identity: string | null,
  cached: T | null,
): HistoryResourceState<T> {
  if (!identity) return idleResource
  if (cached !== null) {
    return { error: null, identity, status: 'loaded', value: cached }
  }
  return { error: null, identity, status: 'loading', value: null }
}
