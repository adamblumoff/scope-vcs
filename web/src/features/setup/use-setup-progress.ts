import type {
  RepoLifecycleState,
  RepoParams,
  SetupProgressState,
} from '@/api/types'
import { useEffect } from 'react'

type SetupProgressCallbacks = {
  onProgressError: (message: string) => void
  onProgressState: (state: SetupProgressState) => void
  onPublished: () => Promise<void> | void
  onReviewReady: () => Promise<void> | void
}

export function useSetupProgress({
  loadProgress,
  onProgressError,
  onProgressState,
  onPublished,
  onReviewReady,
  params,
}: {
  loadProgress: (params: RepoParams) => Promise<RepoLifecycleState>
  params: RepoParams
} & SetupProgressCallbacks) {
  useEffect(() => {
    let cancelled = false
    let inFlight = false

    async function checkProgress() {
      if (inFlight) {
        return
      }

      inFlight = true
      try {
        const lifecycleState = await loadProgress(params)
        if (cancelled) {
          return
        }

        if (lifecycleState === 'PendingPublish') {
          onProgressState('opening-review')
          await onReviewReady()
        } else if (lifecycleState === 'Published') {
          onProgressState('published')
          await onPublished()
        } else {
          onProgressState('waiting')
        }
      } catch (progressError) {
        if (!cancelled) {
          onProgressError(progressErrorMessage(progressError))
        }
      } finally {
        inFlight = false
      }
    }

    const checkOnFocus = () => void checkProgress()
    const checkOnVisibility = () => {
      if (document.visibilityState === 'visible') {
        void checkProgress()
      }
    }
    const initialCheck = window.setTimeout(() => void checkProgress(), 250)
    const interval = window.setInterval(() => void checkProgress(), 1000)
    window.addEventListener('focus', checkOnFocus)
    document.addEventListener('visibilitychange', checkOnVisibility)

    return () => {
      cancelled = true
      window.clearTimeout(initialCheck)
      window.clearInterval(interval)
      window.removeEventListener('focus', checkOnFocus)
      document.removeEventListener('visibilitychange', checkOnVisibility)
    }
  }, [
    loadProgress,
    onProgressError,
    onProgressState,
    onPublished,
    onReviewReady,
    params,
  ])
}

function progressErrorMessage(error: unknown) {
  return error instanceof Error ? error.message : 'setup progress check failed'
}
