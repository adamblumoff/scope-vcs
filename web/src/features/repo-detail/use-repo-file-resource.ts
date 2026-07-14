import type { RepoFileContent } from '@/api/types'
import { useCallback, useEffect, useState } from 'react'
import { readRepoFileCache, writeRepoFileCache } from './repo-file-cache'

type RepoFileResourceState =
  | { error: null; file: null; identity: null; status: 'idle' }
  | { error: null; file: null; identity: string; status: 'loading' }
  | { error: null; file: RepoFileContent; identity: string; status: 'loaded' }
  | { error: string; file: null; identity: string; status: 'error' }

const IDLE_RESOURCE: RepoFileResourceState = {
  error: null,
  file: null,
  identity: null,
  status: 'idle',
}

export function useRepoFileResource({
  identity,
  load,
}: {
  identity: string | null
  load: (signal: AbortSignal) => Promise<RepoFileContent>
}): RepoFileResourceState & { retry: () => void } {
  const [retryVersion, setRetryVersion] = useState(0)
  const [resource, setResource] = useState<RepoFileResourceState>(IDLE_RESOURCE)
  const cachedFile = identity ? readRepoFileCache(identity) : null
  const retry = useCallback(() => setRetryVersion((version) => version + 1), [])

  useEffect(() => {
    if (!identity) {
      setResource(IDLE_RESOURCE)
      return
    }

    const cached = readRepoFileCache(identity)
    if (cached) {
      setResource({ error: null, file: cached, identity, status: 'loaded' })
      return
    }

    const controller = new AbortController()
    let active = true
    setResource({ error: null, file: null, identity, status: 'loading' })

    void load(controller.signal).then(
      (file) => {
        if (!active || controller.signal.aborted) return
        writeRepoFileCache(identity, file)
        setResource({ error: null, file, identity, status: 'loaded' })
      },
      (error: unknown) => {
        if (!active || controller.signal.aborted) return
        setResource({
          error: error instanceof Error && error.message.trim()
            ? error.message
            : 'File content is unavailable.',
          file: null,
          identity,
          status: 'error',
        })
      },
    )

    return () => {
      active = false
      controller.abort()
    }
  }, [identity, load, retryVersion])

  if (!identity) return { ...IDLE_RESOURCE, retry }
  if (cachedFile) {
    return { error: null, file: cachedFile, identity, retry, status: 'loaded' }
  }
  if (resource.identity !== identity) {
    return { error: null, file: null, identity, retry, status: 'loading' }
  }
  return { ...resource, retry }
}
