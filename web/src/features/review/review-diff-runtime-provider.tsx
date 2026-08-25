import {
  WorkerPoolContextProvider,
  type WorkerInitializationRenderOptions,
  type WorkerPoolOptions,
} from '@pierre/diffs/react'
import { type ReactNode, useMemo } from 'react'

const PIERRE_WORKER_HIGHLIGHTER_OPTIONS = {} satisfies WorkerInitializationRenderOptions

export function ReviewDiffRuntimeProvider({ children }: { children: ReactNode }) {
  const workerPoolOptions = useMemo(createPierreWorkerPoolOptions, [])

  if (!workerPoolOptions) {
    return children
  }

  return (
    <WorkerPoolContextProvider
      highlighterOptions={PIERRE_WORKER_HIGHLIGHTER_OPTIONS}
      poolOptions={workerPoolOptions}
    >
      {children}
    </WorkerPoolContextProvider>
  )
}

function createPierreWorkerPoolOptions(): WorkerPoolOptions | null {
  if (typeof Worker === 'undefined') {
    return null
  }

  return {
    poolSize: pierreWorkerPoolSize(),
    workerFactory: () =>
      new Worker(
        new URL('@pierre/diffs/worker/worker-portable.js', import.meta.url),
        { type: 'module' },
      ),
  }
}

function pierreWorkerPoolSize() {
  if (typeof navigator === 'undefined' || !navigator.hardwareConcurrency) {
    return 2
  }
  return Math.min(4, Math.max(1, navigator.hardwareConcurrency))
}
