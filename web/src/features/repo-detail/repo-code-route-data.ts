import type { RepoFileContent } from '@/api/types'

export const DEFAULT_REPO_FILE_PATH = 'README.html'

const REBUILD_RETRY_DELAYS = [0, 250, 500, 1_000, 2_000] as const

export type RepoFileLoadResult =
  | { file: RepoFileContent; status: 'ready' }
  | { status: 'missing' | 'rebuilding' }

type RepoCodeResource<T> = { value: T; error: null } | { value: null; error: string }

// Deferred route data settles independently, so a slow tree cannot hold up the
// addressed file. Errors stay with their existing pane instead of the route.
export async function settleRepoCodeResource<T>(load: Promise<T>): Promise<RepoCodeResource<T>> {
  try {
    return { value: await load, error: null }
  } catch (error) {
    return { value: null, error: error instanceof Error ? error.message : 'Repository content is unavailable.' }
  }
}

export function repoCodeResourceLoader<T>(
  initial: Promise<RepoCodeResource<T>> | null,
  reload: (signal: AbortSignal) => Promise<T>,
) {
  let pending: Promise<RepoCodeResource<T>> | null = initial
  return async (signal: AbortSignal): Promise<T> => {
    signal.throwIfAborted()
    const first = pending
    pending = null
    if (!first) return reload(signal)
    const result = await first
    signal.throwIfAborted()
    if (result.error !== null) throw new Error(result.error)
    return result.value
  }
}

export async function loadRepoFileWhenReady({
  load,
  retryDelays = REBUILD_RETRY_DELAYS,
  signal,
}: {
  load: () => Promise<RepoFileLoadResult>
  retryDelays?: readonly number[]
  signal: AbortSignal
}): Promise<RepoFileContent | null> {
  for (const delay of retryDelays) {
    if (delay > 0) await abortableDelay(delay, signal)
    else throwIfAborted(signal)

    const result = await load()
    if (result.status === 'ready') return result.file
    if (result.status === 'missing') return null
  }

  throw new Error('Repository projection is still rebuilding. Try again shortly.')
}

function abortableDelay(delay: number, signal: AbortSignal) {
  return new Promise<void>((resolve, reject) => {
    throwIfAborted(signal)

    const onAbort = () => {
      clearTimeout(timeout)
      reject(abortError())
    }
    const timeout = setTimeout(() => {
      signal.removeEventListener('abort', onAbort)
      resolve()
    }, delay)
    signal.addEventListener('abort', onAbort, { once: true })
  })
}

function throwIfAborted(signal: AbortSignal) {
  if (signal.aborted) throw abortError()
}

function abortError() {
  const error = new Error('The request was aborted.')
  error.name = 'AbortError'
  return error
}
