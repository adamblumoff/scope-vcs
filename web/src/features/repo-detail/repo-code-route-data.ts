import type { RepoFileContent } from '@/api/types'

export const DEFAULT_REPO_FILE_PATH = 'README.html'

const REBUILD_RETRY_DELAYS = [0, 250, 500, 1_000, 2_000] as const

export type RepoFileLoadResult =
  | { file: RepoFileContent; status: 'ready' }
  | { status: 'missing' | 'rebuilding' }

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
