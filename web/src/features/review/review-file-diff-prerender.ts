import { Worker } from 'node:worker_threads'
import type { ReviewFileDiffWorkerInput } from './review-file-diff-render-contract'
import {
  createReviewFileDiffRenderer,
  type ReviewDiffAdmissionState,
  runReviewFileDiffWorker,
} from './review-file-diff-renderer'

const rendererStateKey = Symbol.for('scope.review-file-diff-renderer-state')
const rendererStateHost = globalThis as unknown as Record<
  symbol,
  ReviewDiffAdmissionState | undefined
>
const sharedRendererState = rendererStateHost[rendererStateKey] ?? { active: 0 }
rendererStateHost[rendererStateKey] = sharedRendererState

export const renderReviewFileDiff = createReviewFileDiffRenderer({
  isolatedRender: runIsolatedReviewFileDiffRender,
  state: sharedRendererState,
})

function runIsolatedReviewFileDiffRender(
  input: ReviewFileDiffWorkerInput,
  deadlineMs: number,
) {
  const workerUrl = import.meta.env.PROD
    ? new URL('../_workers/review-file-diff-render-worker.mjs', import.meta.url)
    : new URL('./review-file-diff-render-worker.ts', import.meta.url)
  const worker = new Worker(
    workerUrl,
    { workerData: input },
  )
  return runReviewFileDiffWorker(worker, deadlineMs)
}
